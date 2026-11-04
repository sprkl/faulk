// Drawing a waveform from peaks (AU-2): the level a zoom reads, the columns' lows and highs, the lanes,
// the guards against a view that is not one, and the shape drawn (recorded from a stand-in context).
import { describe, expect, it } from 'vitest'
import { columnsOf, drawWaveform, levelFor, MIN_LANE, useLanes } from './draw'
import { BLOCK, FACTOR, LEVELS, type Peaks } from './format'

/** Peaks of interleaved samples, computed the plain way (the reference the drawing is checked against). */
function peaksOf(samples: number[], channels: number): Peaks {
  const frames = samples.length / channels
  const levels = Array.from({ length: LEVELS }, (_, level) => {
    const block = BLOCK * FACTOR ** level
    const count = Math.ceil(frames / block)
    const data = new Int8Array(count * 2 * channels)
    for (let i = 0; i < count; i++) {
      for (let c = 0; c < channels; c++) {
        let lo = Infinity
        let hi = -Infinity
        for (let f = i * block; f < Math.min(frames, (i + 1) * block); f++) {
          lo = Math.min(lo, samples[f * channels + c]!)
          hi = Math.max(hi, samples[f * channels + c]!)
        }
        data[(i * channels + c) * 2] = Math.floor(lo * 127)
        data[(i * channels + c) * 2 + 1] = Math.ceil(hi * 127)
      }
    }
    return { block, count, data }
  })
  return { sampleRate: 48_000, channels, frames, levels }
}

/** A context that records the shapes filled: each fill's points. */
function recorder() {
  const fills: { color: string; points: [number, number][] }[] = []
  let points: [number, number][] = []
  const ctx = {
    fillStyle: '',
    beginPath: () => (points = []),
    moveTo: (x: number, y: number) => points.push([x, y]),
    lineTo: (x: number, y: number) => points.push([x, y]),
    closePath: () => undefined,
    fill: () => fills.push({ color: ctx.fillStyle, points }),
  }
  return { ctx: ctx as unknown as CanvasRenderingContext2D, fills }
}

describe('the level a zoom reads', () => {
  const peaks = peaksOf(new Array(100_000).fill(0), 1)
  it('is the coarsest whose block fits in a column', () => {
    expect([1, 100, 256, 1023, 1024, 4095, 4096, 16_383, 16_384, 1e9].map((f) => levelFor(peaks, f))).toEqual([0, 0, 0, 0, 1, 1, 2, 2, 3, 3])
  })
})

describe('the columns of a view', () => {
  // Left: a ramp from -1 to 1 over 2 048 frames; right: a constant 0.5.
  const frames = 2048
  const samples: number[] = []
  for (let f = 0; f < frames; f++) samples.push(-1 + (2 * f) / (frames - 1), 0.5)
  const peaks = peaksOf(samples, 2)

  it('gives each column the lowest low and the highest high of the peaks it covers', () => {
    const low = new Float32Array(4)
    const high = new Float32Array(4)
    // Four columns of 512 frames each, level 0 (two peaks a column).
    columnsOf(peaks, 0, 0, 512, 4, [0], low, high)
    for (let x = 0; x < 4; x++) {
      expect(low[x]).toBe(0 + Math.floor((-1 + (2 * x * 512) / (frames - 1)) * 127))
      expect(high[x]).toBe(0 + Math.ceil((-1 + (2 * (x * 512 + 511)) / (frames - 1)) * 127))
    }
    // The right channel alone, then both merged.
    columnsOf(peaks, 0, 0, 512, 4, [1], low, high)
    expect([...low]).toEqual([63, 63, 63, 63])
    expect([...high]).toEqual([64, 64, 64, 64])
    columnsOf(peaks, 0, 0, 512, 4, [0, 1], low, high)
    expect(low[3]).toBe(63)
    expect(high[0]).toBe(64)
  })

  it('leaves the columns outside the media empty, before its start and after its end', () => {
    const low = new Float32Array(6)
    const high = new Float32Array(6)
    columnsOf(peaks, 0, -1024, 512, 6, [1], low, high)
    expect([...low].map((v) => (Number.isNaN(v) ? null : v))).toEqual([null, null, 63, 63, 63, 63])
    columnsOf(peaks, 0, 1024, 512, 6, [1], low, high)
    expect([...low].map((v) => (Number.isNaN(v) ? null : v))).toEqual([63, 63, null, null, null, null])
  })

  it('reads a peak for a column narrower than a block (zoomed in past level 0)', () => {
    const low = new Float32Array(8)
    const high = new Float32Array(8)
    columnsOf(peaks, 0, 0, 32, 8, [1], low, high)
    expect([...high]).toEqual(new Array(8).fill(64))
  })
})

describe('drawing a waveform', () => {
  const rect = { x: 0, y: 0, width: 100, height: 40 }
  const stereo = peaksOf(
    Array.from({ length: 51_200 * 2 }, (_, i) => (i % 2 === 0 ? Math.sin(i / 50) * 0.8 : 0)),
    2,
  )

  it('draws nothing for a view that is not one, or an empty surface', () => {
    for (const framesPerPixel of [0, -1, NaN, Infinity]) {
      const { ctx, fills } = recorder()
      expect(drawWaveform(ctx, stereo, rect, { from: 0, framesPerPixel }, { color: 'red' })).toBeNull()
      expect(fills).toEqual([])
    }
    for (const r of [
      { ...rect, width: 0 },
      { ...rect, height: 0 },
    ]) {
      const { ctx, fills } = recorder()
      expect(drawWaveform(ctx, stereo, r, { from: 0, framesPerPixel: 512 }, { color: 'red' })).toBeNull()
      expect(fills).toEqual([])
    }
    const { ctx } = recorder()
    expect(drawWaveform(ctx, stereo, rect, { from: NaN, framesPerPixel: 512 }, { color: 'red' })).toBeNull()
  })

  it('splits the channels into lanes when each has room, and merges them otherwise', () => {
    expect(useLanes(stereo, 2 * MIN_LANE)).toBe(true)
    expect(useLanes(stereo, 2 * MIN_LANE - 1)).toBe(false)
    expect(useLanes(stereo, 10, true)).toBe(true)
    expect(useLanes(stereo, 100, false)).toBe(false)
    const tall = recorder()
    drawWaveform(tall.ctx, stereo, rect, { from: 0, framesPerPixel: 512 }, { color: 'red' })
    expect(tall.fills.length).toBe(2)
    const short = recorder()
    drawWaveform(short.ctx, stereo, { ...rect, height: 20 }, { from: 0, framesPerPixel: 512 }, { color: 'red' })
    expect(short.fills.length).toBe(1)
  })

  it('reads the level of the zoom, one column per device pixel, and stays inside its lane', () => {
    const { ctx, fills } = recorder()
    const report = drawWaveform(ctx, stereo, rect, { from: 0, framesPerPixel: 512 }, { color: '#0f0', dpr: 2 })
    // 512 frames a CSS pixel at 2 device pixels a CSS pixel: 256 frames a column, level 0; 200 columns.
    expect(report).toEqual({ level: 0, columns: 200 })
    expect(fills[0]!.color).toBe('#0f0')
    const ys = fills[0]!.points.map(([, y]) => y)
    expect(Math.min(...ys)).toBeGreaterThanOrEqual(0)
    expect(Math.max(...ys)).toBeLessThanOrEqual(20)
    // The sine reaches 0.8: the envelope spans about 0.8 of the lane's half height each side.
    expect(Math.min(...ys)).toBeCloseTo(10 - (102 / 127) * 10, 5)
    expect(drawWaveform(recorder().ctx, stereo, rect, { from: 0, framesPerPixel: 51_200 / 100 }, { color: 'red', dpr: 1 })?.level).toBe(0)
    expect(drawWaveform(recorder().ctx, stereo, rect, { from: 0, framesPerPixel: 4096 }, { color: 'red', dpr: 1 })?.level).toBe(2)
  })

  it('draws silence as a line one device pixel tall', () => {
    const { ctx, fills } = recorder()
    drawWaveform(ctx, stereo, rect, { from: 0, framesPerPixel: 512 }, { color: 'red', dpr: 2 })
    // The right channel is silent: its lane (20..40) holds a band of half a CSS pixel around its middle.
    const ys = fills[1]!.points.map(([, y]) => y)
    expect(Math.min(...ys)).toBeCloseTo(30 - 0.25, 5)
    expect(Math.max(...ys)).toBeCloseTo(30 + 0.25, 5)
  })

  it('draws one shape per run of columns inside the media, nothing past its end', () => {
    const { ctx, fills } = recorder()
    // The media ends at frame 51 200: from 25 600 at 512 frames a pixel, columns 50 to 99 are past it.
    drawWaveform(ctx, stereo, rect, { from: 25_600, framesPerPixel: 512 }, { color: 'red', dpr: 1 })
    const xs = fills[0]!.points.map(([x]) => x)
    expect(Math.max(...xs)).toBe(50)
    const none = recorder()
    drawWaveform(none.ctx, stereo, rect, { from: 1e9, framesPerPixel: 512 }, { color: 'red', dpr: 1 })
    expect(none.fills.every((f) => f.points.length === 0)).toBe(true)
  })
})
