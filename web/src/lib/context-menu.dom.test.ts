import { describe, expect, it } from 'vitest'
import { clampMenu, isMenuKey, keepsBrowserMenu, menuPointOf, nextEnabled, unhandledContextMenu } from './context-menu'

const key = (init: Partial<Pick<KeyboardEvent, 'key' | 'shiftKey' | 'ctrlKey' | 'altKey' | 'metaKey'>>) => ({
  key: '',
  shiftKey: false,
  ctrlKey: false,
  altKey: false,
  metaKey: false,
  ...init,
})

describe('the keys that ask for the menu', () => {
  it('are Shift+F10 and the Menu key, with no other modifier', () => {
    expect(isMenuKey(key({ key: 'F10', shiftKey: true }))).toBe(true)
    expect(isMenuKey(key({ key: 'ContextMenu' }))).toBe(true)
    expect(isMenuKey(key({ key: 'F10' }))).toBe(false)
    expect(isMenuKey(key({ key: 'F10', shiftKey: true, ctrlKey: true }))).toBe(false)
    expect(isMenuKey(key({ key: 'a', shiftKey: true }))).toBe(false)
  })
})

describe('what the page does with a right click nobody handled', () => {
  it('cancels it on a button, a slider, a canvas and the page itself', () => {
    for (const tag of ['button', 'div', 'canvas', 'p']) {
      const element = document.createElement(tag)
      expect(unhandledContextMenu({ target: element, defaultPrevented: false })).toBe('cancel')
    }
    expect(unhandledContextMenu({ target: document.body, defaultPrevented: false })).toBe('cancel')
    expect(unhandledContextMenu({ target: null, defaultPrevented: false })).toBe('cancel')
  })
  it('leaves the browser menu in a field where text is typed', () => {
    const text = document.createElement('input')
    text.type = 'text'
    const number = document.createElement('input')
    number.type = 'number'
    const area = document.createElement('textarea')
    const editable = document.createElement('div')
    editable.setAttribute('contenteditable', 'true')
    const inner = document.createElement('span')
    editable.append(inner)
    for (const target of [text, number, area, editable, inner]) {
      expect(unhandledContextMenu({ target, defaultPrevented: false })).toBe('leave')
      expect(keepsBrowserMenu(target)).toBe(true)
    }
  })
  it('does not keep the browser menu for a checkbox, a range, a button input or a list box', () => {
    for (const type of ['checkbox', 'range', 'button']) {
      const input = document.createElement('input')
      input.type = type
      expect(keepsBrowserMenu(input)).toBe(false)
    }
    expect(keepsBrowserMenu(document.createElement('select'))).toBe(false)
  })
  it('leaves an event a component handled alone', () => {
    expect(unhandledContextMenu({ target: document.createElement('div'), defaultPrevented: true })).toBe('handled')
  })
})

describe('the place of a menu', () => {
  it('moves back inside the window from the right and the bottom, and from the top and the left', () => {
    expect(clampMenu(10, 10, 100, 50, 800, 600)).toEqual({ x: 10, y: 10 })
    expect(clampMenu(790, 590, 100, 50, 800, 600)).toEqual({ x: 696, y: 546 })
    expect(clampMenu(-5, -5, 100, 50, 800, 600)).toEqual({ x: 4, y: 4 })
  })
  it('sits in the middle of a focused element', () => {
    expect(menuPointOf({ left: 10, top: 20, width: 40, height: 10 })).toEqual({ x: 30, y: 25 })
    expect(menuPointOf({ left: 5, top: 10, width: 0, height: 0 })).toEqual({ x: 5, y: 10 })
  })
})

describe('moving through the entries', () => {
  const items = [{ disabled: false }, { disabled: true }, { disabled: false }]
  it('skips the disabled ones and wraps round', () => {
    expect(nextEnabled(items, 0, 1)).toBe(2)
    expect(nextEnabled(items, 2, 1)).toBe(0)
    expect(nextEnabled(items, 0, -1)).toBe(2)
    expect(nextEnabled(items, -1, 1)).toBe(0)
  })
  it('finds none when every entry is disabled', () => {
    expect(nextEnabled([{ disabled: true }], 0, 1)).toBe(-1)
    expect(nextEnabled([], 0, 1)).toBe(-1)
  })
})
