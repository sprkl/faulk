/**
 * Time axis of the shared grid (arrangement, piano roll, automation lanes): ticks <-> pixels,
 * zoom, scroll, snapping and the ruler's marks. Pure functions, no DOM; `TimeGrid.svelte` only
 * draws what they return and forwards input to them. One tempo and one time signature (MVP).
 * View state (zoom, scroll, snap) is local to the page: it is not project data and no command.
 */

import { PPQ, barTicks, beatTicks, type Signature } from '../../../views/transport/time'

/** What the user sees of the time axis: the zoom and the tick at the left edge. */
export interface View {
  /** Pixels per tick (CSS pixels): the zoom. */
  pxPerTick: number
  /** The tick at the left edge of the grid. */
  scrollTick: number
}

/** Smallest zoom the grid accepts before the content is also bounded by its length. */
export const MIN_PX_PER_TICK = 0.001
/** Largest zoom: one tick is one pixel. */
export const MAX_PX_PER_TICK = 1
/** The zoom a grid starts at: 96 px per quarter note. */
export const DEFAULT_PX_PER_TICK = 0.1
/** The default length of a grid, in bars. */
export const DEFAULT_TOTAL_BARS = 200
/** Narrowest gap between two ruler marks, in pixels. */
export const MIN_MARK_PX = 12
/** Narrowest gap between two bar numbers, in pixels. */
export const MIN_LABEL_PX = 44

const finiteOr = (value: number, fallback: number): number => (Number.isFinite(value) ? value : fallback)

export const tickToX = (tick: number, view: View): number => (tick - view.scrollTick) * view.pxPerTick
export const xToTick = (x: number, view: View): number => x / view.pxPerTick + view.scrollTick

/** The length of a grid in ticks. */
export const totalTicksOf = (bars: number, sig: Signature): number => Math.max(1, Math.round(bars)) * barTicks(sig)

/** The least zoom for a grid `widthPx` wide: the whole content fits, and never below `MIN_PX_PER_TICK`. */
export function minPxPerTick(widthPx: number, totalTicks: number): number {
  return Math.min(MAX_PX_PER_TICK, Math.max(MIN_PX_PER_TICK, widthPx / Math.max(1, totalTicks)))
}

/**
 * The view kept inside what exists: zoom between the least zoom of the grid and `MAX_PX_PER_TICK`,
 * scroll between the start and the point where the end touches the right edge. A NaN or an
 * infinity falls back to the default instead of poisoning the view.
 */
export function clampView(view: View, widthPx: number, totalTicks: number): View {
  const width = Math.max(0, finiteOr(widthPx, 0))
  const pxPerTick = Math.min(MAX_PX_PER_TICK, Math.max(minPxPerTick(width, totalTicks), finiteOr(view.pxPerTick, DEFAULT_PX_PER_TICK)))
  const maxScroll = Math.max(0, totalTicks - width / pxPerTick)
  return { pxPerTick, scrollTick: Math.min(maxScroll, Math.max(0, finiteOr(view.scrollTick, 0))) }
}

/** Zooms by `factor` (above 1 magnifies) keeping the tick under `anchorX` where it is. */
export function zoomAt(view: View, factor: number, anchorX: number, widthPx: number, totalTicks: number): View {
  const anchored = xToTick(anchorX, view)
  const pxPerTick = view.pxPerTick * finiteOr(factor, 1)
  // The clamp may refuse part of the zoom: the scroll is solved for the zoom that was kept.
  const zoom = clampView({ pxPerTick, scrollTick: 0 }, widthPx, totalTicks).pxPerTick
  return clampView({ pxPerTick: zoom, scrollTick: anchored - anchorX / zoom }, widthPx, totalTicks)
}

/** Scrolls by `dxPx` pixels (positive moves later in time). */
export function scrollBy(view: View, dxPx: number, widthPx: number, totalTicks: number): View {
  return clampView({ pxPerTick: view.pxPerTick, scrollTick: view.scrollTick + finiteOr(dxPx, 0) / view.pxPerTick }, widthPx, totalTicks)
}

/** The tick range shown, `[from, to]`. */
export function visibleRange(view: View, widthPx: number): [number, number] {
  return [view.scrollTick, view.scrollTick + widthPx / view.pxPerTick]
}

/** The eighth-note triplet in ticks: a quarter note in three (the quantize grid's value). */
const TRIPLET_EIGHTH = PPQ / 3

/**
 * The step of the ruler at this zoom, in ticks: the finest subdivision of the beat that is whole
 * in ticks, then the beat, then 1, 2, 4... bars, whose step is at least `minPx` wide. With a
 * triplet `unit` (#711), the unit's own multiples instead (see `tripletUnit`).
 */
export function gridStep(sig: Signature, pxPerTick: number, minPx: number = MIN_MARK_PX, unit: number | null = null): number {
  const beat = beatTicks(sig)
  const bar = barTicks(sig)
  const candidates: number[] = []
  if (unit !== null && unit > 0) {
    // A triplet unit (#711): its lines, then the eighth triplet, then only the beats and bars that hold a whole number of
    // quarter notes, so that every line drawn is a line of the unit (a binary half beat would read as a wrong grid).
    candidates.push(unit)
    if (TRIPLET_EIGHTH > unit && TRIPLET_EIGHTH % unit === 0) candidates.push(TRIPLET_EIGHTH)
    if (beat % PPQ === 0) candidates.push(beat)
    for (let bars = 1; bars <= 1024; bars *= 2) if ((bar * bars) % PPQ === 0) candidates.push(bar * bars)
  } else {
    for (let divisor = 16; divisor >= 1; divisor /= 2) if (Number.isInteger(beat / divisor)) candidates.push(beat / divisor)
    for (let bars = 1; bars <= 1024; bars *= 2) candidates.push(bar * bars)
  }
  return candidates.find((step) => step * pxPerTick >= minPx) ?? candidates[candidates.length - 1]
}

/** How the pointer snaps: not at all, to the ruler's step at this zoom, or to a fixed unit. The arrangement offers these. */
export const SNAP_SETTINGS = ['off', 'auto', 'bar', 'beat', 'half-beat', 'quarter-beat'] as const
/**
 * The piano roll's snap (#711): the same, then the two triplets of the quantize grid (a quarter note in three and in six),
 * so that a note can be written and moved on a triplet position. The arrangement keeps `SNAP_SETTINGS`.
 */
export const ROLL_SNAP_SETTINGS = [...SNAP_SETTINGS, 'eighth-triplet', 'sixteenth-triplet'] as const
export type SnapSetting = (typeof ROLL_SNAP_SETTINGS)[number]

/** The unit of a triplet setting in ticks (the grid's lines follow it), `null` for any other. */
export function tripletUnit(setting: SnapSetting): number | null {
  if (setting === 'eighth-triplet') return TRIPLET_EIGHTH
  if (setting === 'sixteenth-triplet') return PPQ / 6
  return null
}

/** The snapping step in ticks, or `null` for no snapping. A fixed unit finer than one tick falls back to one tick. */
export function snapStep(setting: SnapSetting, sig: Signature, pxPerTick: number): number | null {
  switch (setting) {
    case 'off':
      return null
    case 'auto':
      return gridStep(sig, pxPerTick)
    case 'bar':
      return barTicks(sig)
    case 'beat':
      return beatTicks(sig)
    case 'half-beat':
      return Math.max(1, beatTicks(sig) / 2)
    case 'quarter-beat':
      return Math.max(1, beatTicks(sig) / 4)
    case 'eighth-triplet':
    case 'sixteenth-triplet':
      return tripletUnit(setting)
  }
}

/**
 * The tick snapped to the nearest multiple of `step` (halfway rounds later), never before the
 * start. Without a step the tick is only rounded to a whole tick and kept at or after the start.
 */
export function snapTick(tick: number, step: number | null): number {
  const safe = finiteOr(tick, 0)
  if (step === null || !(step > 0)) return Math.max(0, Math.round(safe))
  return Math.max(0, Math.round(safe / step) * step)
}

export type MarkKind = 'bar' | 'beat' | 'sub'

export interface Mark {
  tick: number
  /** Position in CSS pixels from the left edge. */
  x: number
  kind: MarkKind
  /** The bar number, when this mark is a bar line the ruler names. */
  label: string | null
}

/**
 * The marks of the ruler for the visible range: one per grid step, a bar line where the step
 * meets a bar, a beat line where it meets a beat, a sub line elsewhere. Bar numbers are named
 * every 1, 2, 4... bars so that two names are always `MIN_LABEL_PX` apart. A triplet `unit`
 * (#711, the piano roll's snap) gives lines of that unit (`gridStep`), with the bar and beat lines of the ruler's own
 * step wherever the unit misses them (at most twice as many marks then). At most
 * `widthPx / MIN_MARK_PX + 2` marks.
 */
export function rulerMarks(view: View, widthPx: number, sig: Signature, totalTicks: number, unit: number | null = null): Mark[] {
  if (unit !== null && unit > 0) {
    // The unit's lines, and the bars and beats of the ruler's own step, which a signature whose beat or bar is not a whole
    // number of quarter notes (7/8, 6/8) would otherwise lose: the bar lines and numbers always show where they fall.
    const lines = marksEvery(gridStep(sig, view.pxPerTick, MIN_MARK_PX, unit), view, widthPx, sig, totalTicks)
    const ticks = new Set(lines.map((m) => m.tick))
    const frame = marksEvery(gridStep(sig, view.pxPerTick), view, widthPx, sig, totalTicks).filter((m) => m.kind !== 'sub' && !ticks.has(m.tick))
    return frame.length === 0 ? lines : [...lines, ...frame].sort((a, b) => a.tick - b.tick)
  }
  return marksEvery(gridStep(sig, view.pxPerTick), view, widthPx, sig, totalTicks)
}

/** One mark per multiple of `step` in the visible range: a bar, a beat or a sub line, bars named as `rulerMarks` says. */
function marksEvery(step: number, view: View, widthPx: number, sig: Signature, totalTicks: number): Mark[] {
  const beat = beatTicks(sig)
  const bar = barTicks(sig)
  let stride = 1
  while (bar * stride * view.pxPerTick < MIN_LABEL_PX && stride < 1 << 20) stride *= 2
  const [from, to] = visibleRange(view, widthPx)
  const marks: Mark[] = []
  const last = Math.min(Math.floor(to / step), Math.floor(totalTicks / step))
  for (let k = Math.max(0, Math.ceil(from / step)); k <= last; k++) {
    const tick = k * step
    const isBar = tick % bar === 0
    const barIndex = tick / bar
    marks.push({
      tick,
      x: tickToX(tick, view),
      kind: isBar ? 'bar' : tick % beat === 0 ? 'beat' : 'sub',
      label: isBar && barIndex % stride === 0 ? String(barIndex + 1) : null,
    })
  }
  return marks
}
