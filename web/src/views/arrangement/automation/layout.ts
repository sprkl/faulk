/**
 * The vertical layout of the arrangement (AM-4), as pure functions. Each track has its row of clips; when the track's
 * automation is unfolded that row is followed by one row per lane the editor shows and by a row that adds a lane.
 * The rows are in the page's coordinates: `top` counts from the first row, under the ruler.
 */

/** The default height of a track's row of clips, in CSS pixels: the `lane` token of DESIGN.md (comfort density, #471). */
export const CLIP_ROW = 44
/** The lowest a track's row may be made: the `lane-compact` token. */
export const CLIP_ROW_MIN = 28
/** The highest a track's row may be made. */
export const CLIP_ROW_MAX = 176

/** A track row's height, whole and between the bounds; an unreadable value gives the default. */
export function clampClipRow(height: number, fallback: number = CLIP_ROW): number {
  if (!Number.isFinite(height)) return fallback
  return Math.min(CLIP_ROW_MAX, Math.max(CLIP_ROW_MIN, Math.round(height)))
}
/** The height of a lane's row. */
export const LANE_ROW = 84
/** The height of the row that adds a lane. */
export const ADD_ROW = 30

export type RowKind = 'track' | 'lane' | 'add'

export interface Row {
  kind: RowKind
  /** The index of the track in the list, 0 first. */
  track: number
  trackId: string
  /** The lane of a `lane` row. */
  lane?: string
  top: number
  height: number
}

export interface Layout {
  rows: readonly Row[]
  /** The top of each track's row of clips. */
  trackTop: readonly number[]
  /** The height of each track's row of clips. */
  trackHeight: readonly number[]
  /** The height of all the rows. */
  total: number
}

/**
 * The rows of the tracks, in order. `lanes` gives the ids of the lanes a track shows, in the order they are drawn; only
 * the tracks in `open` use it. `heightOf` gives the height of a track's row of clips (the user's, kept by the page), else
 * `base` (the `lane` token); either is clamped to the bounds.
 */
export function layoutOf(
  entries: readonly { id: string }[],
  open: ReadonlySet<string>,
  lanes: (trackId: string) => readonly string[],
  heightOf: (trackId: string) => number | undefined = () => undefined,
  base: number = CLIP_ROW,
): Layout {
  const rows: Row[] = []
  const trackTop: number[] = []
  const trackHeight: number[] = []
  const fallback = clampClipRow(base)
  let top = 0
  entries.forEach((entry, track) => {
    const height = clampClipRow(heightOf(entry.id) ?? fallback, fallback)
    trackTop.push(top)
    trackHeight.push(height)
    rows.push({ kind: 'track', track, trackId: entry.id, top, height })
    top += height
    if (!open.has(entry.id)) return
    for (const lane of lanes(entry.id)) {
      rows.push({ kind: 'lane', track, trackId: entry.id, lane, top, height: LANE_ROW })
      top += LANE_ROW
    }
    rows.push({ kind: 'add', track, trackId: entry.id, top, height: ADD_ROW })
    top += ADD_ROW
  })
  return { rows, trackTop, trackHeight, total: top }
}

/** The index of the row under `y` (counted from the first row), or -1 above the first row or under the last. */
export function rowIndexAt(layout: Layout, y: number): number {
  if (!(y >= 0) || y >= layout.total) return -1
  let lo = 0
  let hi = layout.rows.length - 1
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1
    if (layout.rows[mid]!.top <= y) lo = mid
    else hi = mid - 1
  }
  return lo
}

/** The row under `y`, or null. */
export function rowAtY(layout: Layout, y: number): Row | null {
  const i = rowIndexAt(layout, y)
  return i < 0 ? null : layout.rows[i]!
}

/** The rows that touch the span from `from` to `to`, in order. */
export function rowsBetween(layout: Layout, from: number, to: number): Row[] {
  const first = Math.max(0, rowIndexAt(layout, Math.max(0, from)))
  const out: Row[] = []
  for (let i = first; i < layout.rows.length; i++) {
    const row = layout.rows[i]!
    if (row.top > to) break
    if (row.top + row.height >= from) out.push(row)
  }
  return out
}

/** The row of a lane, or null. */
export const laneRow = (layout: Layout, lane: string): Row | null => layout.rows.find((r) => r.kind === 'lane' && r.lane === lane) ?? null
