import { describe, expect, it } from 'vitest'
import { itemAt, moveCursor, reading, rovingKey, step, timelineCursorStep, zoomKey, type Item } from './roving'

const item = (id: string, row: number, start: number, end: number): Item => ({ id, row, start, end })
// Row 0: a 0..4, b 4..8; row 1: nothing; row 2: c 2..6, d 8..12; row 3: e 0..1.
const items = [item('b', 0, 4, 8), item('d', 2, 8, 12), item('a', 0, 0, 4), item('c', 2, 2, 6), item('e', 3, 0, 1)]
const get = (id: string) => items.find((i) => i.id === id)!

describe('the roving focus over items in rows (#471)', () => {
  it('reads the items by row, then time', () => {
    expect(reading(items).map((i) => i.id)).toEqual(['a', 'b', 'c', 'd', 'e'])
  })

  it('goes left and right in the same row, and stops at its ends', () => {
    expect(step(items, get('a'), 'right')?.id).toBe('b')
    expect(step(items, get('b'), 'left')?.id).toBe('a')
    expect(step(items, get('a'), 'left')).toBeNull()
    expect(step(items, get('b'), 'right')).toBeNull()
  })

  it('goes up and down to the nearest row that holds an item, to the item closest in time', () => {
    // From b (starts at 4) down: row 1 is empty, row 2 holds c (2..6, which covers 4) and d.
    expect(step(items, get('b'), 'down')?.id).toBe('c')
    expect(step(items, get('a'), 'down')?.id).toBe('c')
    expect(step(items, get('d'), 'up')?.id).toBe('b')
    expect(step(items, get('c'), 'down')?.id).toBe('e')
    expect(step(items, get('e'), 'down')).toBeNull()
    expect(step(items, get('a'), 'up')).toBeNull()
  })

  it('goes left and right across the rows in time order when asked (the notes of a clip)', () => {
    // In time: a(0, row 0), e(0, row 3), c(2), b(4), d(8).
    expect(step(items, get('a'), 'right', true)?.id).toBe('e')
    expect(step(items, get('e'), 'right', true)?.id).toBe('c')
    expect(step(items, get('b'), 'left', true)?.id).toBe('c')
    expect(step(items, get('d'), 'right', true)).toBeNull()
    expect(step(items, get('a'), 'left', true)).toBeNull()
  })

  it('finds nothing from an item that is gone', () => {
    expect(step(items, item('zz', 0, 0, 1), 'right')).toBeNull()
  })

  it('reads Ctrl or Cmd with plus and minus as the zoom, and nothing else', () => {
    const k = (key: string, over: Partial<KeyboardEvent> = {}) => zoomKey({ key, altKey: false, ctrlKey: true, metaKey: false, ...over } as KeyboardEvent)
    expect(k('=')).toBe(1)
    expect(k('+')).toBe(1)
    expect(k('-')).toBe(-1)
    expect(k('_')).toBe(-1)
    expect(k('-', { ctrlKey: false, metaKey: true })).toBe(-1)
    expect(k('-', { ctrlKey: false })).toBeNull()
    expect(k('-', { altKey: true })).toBeNull()
    expect(k('a')).toBeNull()
  })

  it('maps the keys: arrows focus, Shift extends, Alt moves, Enter opens, Delete removes', () => {
    const k = (key: string, over: Partial<KeyboardEvent> = {}) =>
      rovingKey({ key, shiftKey: false, altKey: false, ctrlKey: false, metaKey: false, ...over } as KeyboardEvent)
    expect(k('ArrowLeft')).toEqual({ kind: 'focus', direction: 'left' })
    expect(k('ArrowDown', { shiftKey: true })).toEqual({ kind: 'extend', direction: 'down' })
    expect(k('ArrowRight', { altKey: true })).toEqual({ kind: 'move', direction: 'right' })
    expect(k('Enter')).toEqual({ kind: 'open' })
    expect(k('Delete')).toEqual({ kind: 'delete' })
    expect(k('Backspace')).toEqual({ kind: 'delete' })
    expect(k('ArrowLeft', { ctrlKey: true })).toBeNull()
    expect(k('Enter', { shiftKey: true })).toBeNull()
    expect(k('a')).toBeNull()
  })

  it('maps Escape to freeing the cursor, and Page Up and Page Down to the velocity, by 10 or by 1 with Shift (#494)', () => {
    const k = (key: string, over: Partial<KeyboardEvent> = {}) =>
      rovingKey({ key, shiftKey: false, altKey: false, ctrlKey: false, metaKey: false, ...over } as KeyboardEvent)
    expect(k('Escape')).toEqual({ kind: 'free' })
    expect(k('Escape', { shiftKey: true })).toBeNull()
    expect(k('PageUp')).toEqual({ kind: 'velocity', delta: 10 })
    expect(k('PageDown')).toEqual({ kind: 'velocity', delta: -10 })
    expect(k('PageUp', { shiftKey: true })).toEqual({ kind: 'velocity', delta: 1 })
    expect(k('PageDown', { shiftKey: true })).toEqual({ kind: 'velocity', delta: -1 })
    // Alt+Shift+arrows stay the octave transposition; Alt or Ctrl with Page Up is not the velocity.
    expect(k('ArrowUp', { altKey: true, shiftKey: true })).toEqual({ kind: 'move', direction: 'up' })
    expect(k('PageUp', { altKey: true })).toBeNull()
    expect(k('PageUp', { ctrlKey: true })).toBeNull()
  })
})

describe('the free cursor (#494)', () => {
  const steps = { tick: 240, row: 1 }
  it('moves by the step in time and by rows, and stops at the edges', () => {
    expect(moveCursor({ row: 2, tick: 0 }, 'right', steps, 4, 960)).toEqual({ row: 2, tick: 240 })
    expect(moveCursor({ row: 2, tick: 240 }, 'left', steps, 4, 960)).toEqual({ row: 2, tick: 0 })
    expect(moveCursor({ row: 2, tick: 0 }, 'left', steps, 4, 960)).toEqual({ row: 2, tick: 0 })
    // The last step of the range: 720 starts a step that ends at 960.
    expect(moveCursor({ row: 2, tick: 720 }, 'right', steps, 4, 960)).toEqual({ row: 2, tick: 720 })
    expect(moveCursor({ row: 2, tick: 0 }, 'up', steps, 4, 960)).toEqual({ row: 1, tick: 0 })
    expect(moveCursor({ row: 0, tick: 0 }, 'up', steps, 4, 960)).toEqual({ row: 0, tick: 0 })
    expect(moveCursor({ row: 3, tick: 0 }, 'down', steps, 4, 960)).toEqual({ row: 3, tick: 0 })
    expect(moveCursor({ row: 0, tick: 0 }, 'down', { tick: 240, row: 12 }, 128, 960)).toEqual({ row: 12, tick: 0 })
    expect(moveCursor({ row: 120, tick: 0 }, 'down', { tick: 240, row: 12 }, 128, 960)).toEqual({ row: 127, tick: 0 })
  })

  it('goes back on the step grid from a tick between two steps', () => {
    expect(moveCursor({ row: 0, tick: 300 }, 'left', steps, 1, 960)).toEqual({ row: 0, tick: 240 })
    expect(moveCursor({ row: 0, tick: 300 }, 'right', steps, 1, 960)).toEqual({ row: 0, tick: 480 })
  })

  it('stays put in a range shorter than a step, or with an empty one', () => {
    expect(moveCursor({ row: 0, tick: 0 }, 'right', { tick: 3840, row: 1 }, 1, 960)).toEqual({ row: 0, tick: 0 })
    expect(moveCursor({ row: 0, tick: 0 }, 'down', steps, 0, 960)).toEqual({ row: 0, tick: 0 })
  })

  it('steps the timeline cursor by the snap, never finer than a beat, and by a bar with the snap off', () => {
    // 4/4: a beat of 960, a bar of 3840. Auto at the default zoom snaps to 120 (a 1/32): the cursor still goes by a beat.
    expect(timelineCursorStep(120, 960, 3840)).toBe(960)
    expect(timelineCursorStep(960, 960, 3840)).toBe(960)
    expect(timelineCursorStep(1920, 960, 3840)).toBe(1920)
    expect(timelineCursorStep(3840, 960, 3840)).toBe(3840)
    expect(timelineCursorStep(null, 960, 3840)).toBe(3840)
    // 6/8: an eighth-note beat of 480.
    expect(timelineCursorStep(240, 480, 2880)).toBe(480)
  })

  it('finds the item under the cursor, the latest started first', () => {
    expect(itemAt(items, { row: 2, tick: 4 })?.id).toBe('c')
    expect(itemAt(items, { row: 2, tick: 6 })).toBeNull()
    expect(itemAt(items, { row: 0, tick: 4 })?.id).toBe('b')
    expect(itemAt([...items, item('f', 2, 3, 5)], { row: 2, tick: 4 })?.id).toBe('f')
    expect(itemAt(items, { row: 1, tick: 0 })).toBeNull()
  })
})
