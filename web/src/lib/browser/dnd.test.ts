// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import type { UiStore } from '../stores/ui.svelte'
import type { BrowserItem } from './catalog'
import { DRAG_TYPE, dropTarget, endDrag, insertionIndex, startDrag, type CardBox } from './dnd'

const card = (index: number, left: number, top: number, w = 100, h = 80): CardBox => ({ index, left, top, right: left + w, bottom: top + h })

describe('where a drop falls in a chain', () => {
  const row = [card(0, 0, 0), card(1, 110, 0), card(2, 220, 0)]

  it('is the first card whose middle is right of the pointer', () => {
    expect(insertionIndex(row, 10, 40)).toBe(0)
    expect(insertionIndex(row, 60, 40)).toBe(1)
    expect(insertionIndex(row, 120, 40)).toBe(1)
    expect(insertionIndex(row, 200, 40)).toBe(2)
  })

  it('is the end of the chain past the last card, and 0 for a chain with no effect', () => {
    expect(insertionIndex(row, 330, 40)).toBe(3)
    expect(insertionIndex([], 10, 10)).toBe(0)
  })

  it('reads wrapped rows top to bottom: above a row is before its first card, below all is the end', () => {
    const wrapped = [...row, card(3, 0, 100), card(4, 110, 100)]
    expect(insertionIndex(wrapped, 400, 50)).toBe(3)
    expect(insertionIndex(wrapped, 10, 140)).toBe(3)
    expect(insertionIndex(wrapped, 200, 140)).toBe(5)
    expect(insertionIndex(wrapped, 10, 95)).toBe(3)
  })
})

describe('a drop target', () => {
  const item: BrowserItem = { kind: 'faulk.reverb', name: 'Reverb', category: 'effect', preset: null }
  const ui = (dragged: BrowserItem | null) => ({ dragged }) as unknown as UiStore
  const event = (type: string, init: { files?: boolean } = {}) => {
    const e = new Event(type, { bubbles: true, cancelable: true }) as DragEvent
    Object.defineProperty(e, 'dataTransfer', {
      value: { types: init.files ? ['Files'] : [], files: init.files ? [new File([''], 'a.wav')] : [], dropEffect: 'none' },
    })
    return e
  }

  it('takes the dragover of an accepted item and not of a refused one, and marks itself while over', () => {
    const node = document.createElement('div')
    const store = ui(item)
    const a = dropTarget(node, { ui: store, accepts: () => true, drop: vi.fn() })
    const over = event('dragover')
    node.dispatchEvent(event('dragenter'))
    node.dispatchEvent(over)
    expect(over.defaultPrevented).toBe(true)
    expect(node.hasAttribute('data-drop-over')).toBe(true)
    node.dispatchEvent(event('dragleave'))
    expect(node.hasAttribute('data-drop-over')).toBe(false)
    a.destroy()

    const refusing = dropTarget(node, { ui: store, accepts: () => false, drop: vi.fn() })
    const refused = event('dragover')
    node.dispatchEvent(event('dragenter'))
    node.dispatchEvent(refused)
    expect(refused.defaultPrevented).toBe(false)
    expect(node.hasAttribute('data-drop-over')).toBe(false)
    refusing.destroy()
  })

  it('drops once, clears the drag, and the target inside wins over the one around it', () => {
    const outer = document.createElement('div')
    const inner = document.createElement('div')
    outer.append(inner)
    const store = ui(item)
    const dropOuter = vi.fn()
    const dropInner = vi.fn()
    dropTarget(outer, { ui: store, accepts: () => true, drop: dropOuter })
    dropTarget(inner, { ui: store, accepts: () => true, drop: dropInner })
    inner.dispatchEvent(event('dragenter'))
    inner.dispatchEvent(event('drop'))
    expect(dropInner).toHaveBeenCalledTimes(1)
    expect(dropOuter).not.toHaveBeenCalled()
    expect(store.dragged).toBeNull()
    expect(outer.hasAttribute('data-drop-over')).toBe(false)
    expect(inner.hasAttribute('data-drop-over')).toBe(false)
  })

  it('ignores a drop when nothing of the browser is dragged (a drag from elsewhere)', () => {
    const node = document.createElement('div')
    const drop = vi.fn()
    dropTarget(node, { ui: ui(null), accepts: () => true, drop })
    const over = event('dragover')
    node.dispatchEvent(over)
    node.dispatchEvent(event('drop'))
    expect(over.defaultPrevented).toBe(false)
    expect(drop).not.toHaveBeenCalled()
  })

  it('hands files to a target that takes them, and only to it', () => {
    const node = document.createElement('div')
    const files = vi.fn()
    const drop = vi.fn()
    dropTarget(node, { ui: ui(null), accepts: () => true, drop, files })
    const over = event('dragover', { files: true })
    node.dispatchEvent(over)
    node.dispatchEvent(event('drop', { files: true }))
    expect(over.defaultPrevented).toBe(true)
    expect(files).toHaveBeenCalledTimes(1)
    expect(drop).not.toHaveBeenCalled()

    const plain = document.createElement('div')
    const nope = dropTarget(plain, { ui: ui(null), accepts: () => true, drop })
    const refused = event('dragover', { files: true })
    plain.dispatchEvent(refused)
    expect(refused.defaultPrevented).toBe(false)
    nope.destroy()
  })
})

describe('a drag out of the browser', () => {
  const item: BrowserItem = {
    kind: 'faulk.reverb',
    name: 'Reverb',
    category: 'effect',
    preset: { ref: { location: 'factory', id: 'faulk.reverb/large-hall' }, name: 'Large Hall', category: null },
  }

  it('is kept for the targets while it moves, carries the preset and the name, and ends with the drag', () => {
    const ui = { dragged: null } as unknown as UiStore
    const data = new Map<string, string>()
    const event = { dataTransfer: { setData: (type: string, value: string) => data.set(type, value), effectAllowed: '' } } as unknown as DragEvent
    startDrag(ui, event, item)
    expect(ui.dragged).toBe(item)
    expect(JSON.parse(data.get(DRAG_TYPE)!)).toEqual({ kind: 'faulk.reverb', preset: { location: 'factory', id: 'faulk.reverb/large-hall' } })
    expect(data.get('text/plain')).toBe('Reverb, Large Hall')
    endDrag(ui)
    expect(ui.dragged).toBeNull()
  })

  it('still marks the drag when the browser gives no data transfer', () => {
    const ui = { dragged: null } as unknown as UiStore
    startDrag(ui, {} as DragEvent, item)
    expect(ui.dragged).toBe(item)
  })
})
