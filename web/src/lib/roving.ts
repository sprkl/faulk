/**
 * Keyboard moves over items laid out in rows along time (#471): the clips of the arrangement (a row is a track) and
 * the notes of the piano roll (a row is a pitch). Pure functions, no DOM: the views keep one item in the tab order
 * (roving focus) and ask here where an arrow goes.
 */

export interface Item {
  id: string
  /** The row, 0 first (a track's index; for notes, a row index where 0 is the highest pitch shown first). */
  row: number
  start: number
  end: number
}

export type Direction = 'left' | 'right' | 'up' | 'down'

/** Items ordered as they read: by row, then by start, then by id. */
export function reading<T extends Item>(items: readonly T[]): T[] {
  return [...items].sort((a, b) => a.row - b.row || a.start - b.start || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
}

/**
 * The item an arrow goes to from `from`, or null at the edge. Left and Right go to the previous or next item of the same
 * row in time (with `byTime`, of all the rows); Up and Down to the nearest row above or below that holds an item, and in it to the item closest in time
 * to `from`'s start (an item that overlaps that time first).
 */
export function step<T extends Item>(items: readonly T[], from: T, direction: Direction, byTime = false): T | null {
  const sorted = reading(items)
  if ((direction === 'left' || direction === 'right') && byTime) {
    // Across the rows, in the order of time (the notes of a clip: a chord's notes from the top down).
    const inTime = [...items].sort((a, b) => a.start - b.start || a.row - b.row || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    const at = inTime.findIndex((i) => i.id === from.id)
    if (at < 0) return null
    return inTime[direction === 'left' ? at - 1 : at + 1] ?? null
  }
  if (direction === 'left' || direction === 'right') {
    const row = sorted.filter((i) => i.row === from.row)
    const at = row.findIndex((i) => i.id === from.id)
    if (at < 0) return null
    return row[direction === 'left' ? at - 1 : at + 1] ?? null
  }
  const rows = [...new Set(sorted.map((i) => i.row))]
  const candidates = direction === 'up' ? rows.filter((r) => r < from.row).reverse() : rows.filter((r) => r > from.row)
  const target = candidates[0]
  if (target === undefined) return null
  let best: T | null = null
  let bestDistance = Number.POSITIVE_INFINITY
  for (const item of sorted) {
    if (item.row !== target) continue
    const distance = from.start >= item.start && from.start < item.end ? 0 : Math.abs(item.start - from.start)
    if (distance < bestDistance) {
      best = item
      bestDistance = distance
    }
  }
  return best
}

/**
 * Ctrl or Cmd with plus or minus on a cell: the grid's zoom (in: 1, out: -1), as on the grid itself, and never the page's.
 * Null for any other key.
 */
export function zoomKey(event: Pick<KeyboardEvent, 'key' | 'altKey' | 'ctrlKey' | 'metaKey'>): 1 | -1 | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return null
  if (event.key === '+' || event.key === '=') return 1
  if (event.key === '-' || event.key === '_') return -1
  return null
}

/**
 * The keys of the roving focus: an arrow, with what it does; Enter, Delete, Escape (frees the cursor at the item, #494), and
 * Page Up or Page Down (the selection's velocity, #494: by 10, by 1 with Shift).
 */
export type RovingKey =
  | { kind: 'focus' | 'extend' | 'move'; direction: Direction }
  | { kind: 'open' }
  | { kind: 'delete' }
  | { kind: 'free' }
  | { kind: 'velocity'; delta: number }
  | null

const ARROWS: Record<string, Direction> = { ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down' }

/** The step of Page Up and Page Down on the velocity, and with Shift (#494). */
export const VELOCITY_STEP = 10
export const VELOCITY_FINE_STEP = 1

/**
 * Arrows move the focus, Shift+arrows extend the selection, Alt+arrows move the item; Enter opens (creates, on the free
 * cursor); Delete and Backspace remove; Escape frees the cursor; Page Up and Page Down change the velocity (Shift: by 1).
 */
export function rovingKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'altKey' | 'ctrlKey' | 'metaKey'>): RovingKey {
  if (event.ctrlKey || event.metaKey) return null
  const direction = ARROWS[event.key]
  if (direction) return { kind: event.altKey ? 'move' : event.shiftKey ? 'extend' : 'focus', direction }
  if (event.altKey) return null
  if (event.key === 'PageUp' || event.key === 'PageDown')
    return { kind: 'velocity', delta: (event.key === 'PageUp' ? 1 : -1) * (event.shiftKey ? VELOCITY_FINE_STEP : VELOCITY_STEP) }
  if (event.shiftKey) return null
  if (event.key === 'Enter') return { kind: 'open' }
  if (event.key === 'Delete' || event.key === 'Backspace') return { kind: 'delete' }
  if (event.key === 'Escape') return { kind: 'free' }
  return null
}

/** The free cursor (#494): a row (a track, or a pitch row where 0 is the highest pitch) and a tick, where no item needs to be. */
export interface Cursor {
  row: number
  tick: number
}

/**
 * Where an arrow takes the free cursor: Left and Right by `steps.tick` (kept on that step's grid inside `[0, end)`), Up and
 * Down by `steps.row` rows (kept in `[0, rows)`). A step past an edge stops at the edge, so the cursor never leaves the grid.
 */
export function moveCursor(cursor: Cursor, direction: Direction, steps: { tick: number; row: number }, rows: number, end: number): Cursor {
  if (direction === 'up' || direction === 'down') {
    const row = cursor.row + (direction === 'up' ? -steps.row : steps.row)
    return { row: Math.min(Math.max(0, row), Math.max(0, rows - 1)), tick: cursor.tick }
  }
  const step = Math.max(1, Math.round(steps.tick))
  const last = Math.max(0, Math.floor((Math.max(1, end) - 1) / step) * step)
  const from = Math.floor(Math.max(0, cursor.tick) / step) * step
  if (direction === 'left') return { row: cursor.row, tick: Math.max(0, from < cursor.tick ? from : from - step) }
  // Off the grid (a clip's start between two steps), Right first goes to the next step.
  return from + step > last ? cursor : { row: cursor.row, tick: from + step }
}

/**
 * The step of the timeline's cursor along time (#494): the snap step, never finer than a beat (a fine snap would take dozens
 * of presses per bar), and a bar with the snap off.
 */
export function timelineCursorStep(snap: number | null, beat: number, bar: number): number {
  return Math.max(snap ?? bar, beat)
}

/** A jump of the free cursor along time (#771): to the start or the end of the clip, or one bar earlier or later. */
export type CursorJump = 'start' | 'end' | 'barBack' | 'barForward'

/**
 * The keys that make the free cursor jump (#771): Home and End (to the start and the end of the clip), Ctrl or Cmd + Left
 * and Right (one bar). Null for any other key, and with Alt or Shift held.
 */
export function cursorJumpKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'altKey' | 'ctrlKey' | 'metaKey'>): CursorJump | null {
  if (event.altKey || event.shiftKey) return null
  const mod = event.ctrlKey || event.metaKey
  if (!mod && event.key === 'Home') return 'start'
  if (!mod && event.key === 'End') return 'end'
  if (mod && event.key === 'ArrowLeft') return 'barBack'
  if (mod && event.key === 'ArrowRight') return 'barForward'
  return null
}

/**
 * Where a jump takes the cursor's tick (#771), whatever the step: the start of the clip, the last `step` line before its
 * `end`, or `bar` ticks earlier or later, kept on the `step` grid inside `[0, last]` (a jump past an edge stops there).
 */
export function jumpCursor(tick: number, jump: CursorJump, steps: { step: number; bar: number }, end: number): number {
  const step = Math.max(1, Math.round(steps.step))
  const last = Math.max(0, Math.floor((Math.max(1, end) - 1) / step) * step)
  if (jump === 'start') return 0
  if (jump === 'end') return last
  const to = tick + (jump === 'barBack' ? -1 : 1) * Math.max(1, Math.round(steps.bar))
  return Math.min(last, Math.max(0, Math.floor(to / step) * step))
}

/** The item under the cursor: in its row, the one whose span holds its tick (the latest start first), or null. */
export function itemAt<T extends Item>(items: readonly T[], cursor: Cursor): T | null {
  let found: T | null = null
  for (const item of items) {
    if (item.row !== cursor.row || cursor.tick < item.start || cursor.tick >= item.end) continue
    if (!found || item.start > found.start) found = item
  }
  return found
}
