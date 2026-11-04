// Drawing a waveform from its peaks (AU-2), on any canvas: the `Waveform` component's own, or a
// surface that draws many clips at once (the arrangement's time grid, AU-6). One column per device
// pixel, from the coarsest level whose block still fits in a column, so a column reads at most a few
// peaks at any zoom: up to 65 536 frames a device pixel, four peaks or fewer; beyond, the top level's
// (16 384 frames) peaks in a column grow with the zoom-out (an hour of 48 kHz in 1 000 pixels: about
// eleven), never with the media's length.
import type { Peaks } from './format'

/** What part of the media a surface shows: the frame at its left edge, and how many frames a CSS pixel holds. */
export interface WaveformView {
  from: number
  framesPerPixel: number
}

export interface WaveformStyle {
  /** A colour a canvas takes (not `currentColor`). */
  color: string
  /** One lane per channel, or the channels merged into one; `auto`: lanes when each would be at least `MIN_LANE` px tall. */
  lanes?: boolean | 'auto'
  dpr?: number
}

/** The smallest lane, in CSS pixels, below which `auto` merges the channels. */
export const MIN_LANE = 12

/** The coarsest level whose block is no wider than `framesPerColumn` (level 0 when zoomed in further). */
export function levelFor(peaks: Peaks, framesPerColumn: number): number {
  let level = 0
  while (level + 1 < peaks.levels.length && peaks.levels[level + 1]!.block <= framesPerColumn) level++
  return level
}

/** Whether `auto` lanes split the channels in a surface `height` px tall. */
export function useLanes(peaks: Peaks, height: number, lanes: boolean | 'auto' = 'auto'): boolean {
  if (lanes !== 'auto') return lanes && peaks.channels > 1
  return peaks.channels > 1 && height / peaks.channels >= MIN_LANE
}

/** The low and high (in -127..127) of `channels` over `columns` columns of `framesPerColumn` frames from `from`; columns outside the media are `NaN`. */
export function columnsOf(
  peaks: Peaks,
  level: number,
  from: number,
  framesPerColumn: number,
  columns: number,
  channels: readonly number[],
  low: Float32Array,
  high: Float32Array,
): void {
  const { block, count, data } = peaks.levels[level]!
  const stride = 2 * peaks.channels
  for (let x = 0; x < columns; x++) {
    const f0 = from + x * framesPerColumn
    const f1 = f0 + framesPerColumn
    if (f1 <= 0 || f0 >= peaks.frames) {
      low[x] = NaN
      high[x] = NaN
      continue
    }
    const i0 = Math.max(0, Math.floor(f0 / block))
    const i1 = Math.min(count, Math.max(i0 + 1, Math.ceil(f1 / block)))
    let lo = 127
    let hi = -127
    for (let i = i0; i < i1; i++) {
      for (const c of channels) {
        const l = data[i * stride + 2 * c]!
        const h = data[i * stride + 2 * c + 1]!
        if (l < lo) lo = l
        if (h > hi) hi = h
      }
    }
    low[x] = lo
    high[x] = hi
  }
}

/** The middle of a column's envelope, in CSS pixels: what a column thinner than a device pixel grows around. */
const centre = (i: number, mid: number, half: number) => mid - ((scratchHigh[i]! + scratchLow[i]!) / 254) * half

let scratchLow = new Float32Array(0)
let scratchHigh = new Float32Array(0)

/** What a draw did, for the browser checks. */
export interface DrawReport {
  level: number
  columns: number
}

/**
 * Draws `peaks` into `rect` (CSS pixels of a context scaled by `dpr`) as `view` places it: one filled
 * envelope per lane, at least one device pixel tall so silence is a line. Draws nothing for a view
 * that is not one (a zoom of 0, NaN) or an empty rectangle.
 */
export function drawWaveform(
  ctx: CanvasRenderingContext2D,
  peaks: Peaks,
  rect: { x: number; y: number; width: number; height: number },
  view: WaveformView,
  style: WaveformStyle,
): DrawReport | null {
  const dpr = style.dpr && style.dpr > 0 ? style.dpr : 1
  const columns = Math.ceil(rect.width * dpr)
  const framesPerColumn = view.framesPerPixel / dpr
  if (!(columns > 0 && rect.height > 0 && framesPerColumn > 0 && Number.isFinite(framesPerColumn) && Number.isFinite(view.from))) return null
  if (columns > 1 << 16) return null
  const level = levelFor(peaks, framesPerColumn)
  if (scratchLow.length < columns) {
    scratchLow = new Float32Array(columns)
    scratchHigh = new Float32Array(columns)
  }
  const lanes = useLanes(peaks, rect.height, style.lanes)
  const groups = lanes ? Array.from({ length: peaks.channels }, (_, c) => [c]) : [Array.from({ length: peaks.channels }, (_, c) => c)]
  const laneHeight = rect.height / groups.length
  const px = 1 / dpr
  ctx.fillStyle = style.color
  for (const [lane, channels] of groups.entries()) {
    columnsOf(peaks, level, view.from, framesPerColumn, columns, channels, scratchLow, scratchHigh)
    const mid = rect.y + laneHeight * (lane + 0.5)
    const half = laneHeight / 2
    ctx.beginPath()
    let x = 0
    while (x < columns) {
      // One closed shape per run of columns inside the media: along the highs, back along the lows.
      while (x < columns && Number.isNaN(scratchLow[x]!)) x++
      if (x >= columns) break
      const start = x
      while (x < columns && !Number.isNaN(scratchLow[x]!)) x++
      for (let i = start; i < x; i++) {
        const top = Math.min(mid - (scratchHigh[i]! / 127) * half, centre(i, mid, half) - px / 2)
        const left = rect.x + i * px
        if (i === start) ctx.moveTo(left, top)
        else ctx.lineTo(left, top)
        ctx.lineTo(left + px, top)
      }
      for (let i = x - 1; i >= start; i--) {
        const bottom = Math.max(mid - (scratchLow[i]! / 127) * half, centre(i, mid, half) + px / 2)
        const left = rect.x + i * px
        ctx.lineTo(left + px, bottom)
        ctx.lineTo(left, bottom)
      }
      ctx.closePath()
    }
    ctx.fill()
  }
  return { level, columns }
}
