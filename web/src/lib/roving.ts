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

/** The keys of the roving focus: an arrow, with what it does. */
export type RovingKey = { kind: 'focus' | 'extend' | 'move'; direction: Direction } | { kind: 'open' } | { kind: 'delete' } | null

const ARROWS: Record<string, Direction> = { ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down' }

/** Arrows move the focus, Shift+arrows extend the selection, Alt+arrows move the item; Enter opens; Delete and Backspace remove. */
export function rovingKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'altKey' | 'ctrlKey' | 'metaKey'>): RovingKey {
  if (event.ctrlKey || event.metaKey) return null
  const direction = ARROWS[event.key]
  if (direction) return { kind: event.altKey ? 'move' : event.shiftKey ? 'extend' : 'focus', direction }
  if (event.altKey || event.shiftKey) return null
  if (event.key === 'Enter') return { kind: 'open' }
  if (event.key === 'Delete' || event.key === 'Backspace') return { kind: 'delete' }
  return null
}
