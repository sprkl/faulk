import { describe, expect, it } from 'vitest'
import { reading, rovingKey, step, type Item } from './roving'

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
})
