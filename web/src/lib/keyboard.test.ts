// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { clampOctave, historyShortcut, ignoreKeyEvent, noteForKey, octaveStep } from './keyboard'

describe('computer keyboard', () => {
  it('maps physical keys to notes', () => {
    expect(noteForKey('KeyA', 4)).toBe(60)
    expect(noteForKey('KeyW', 4)).toBe(61)
    expect(noteForKey('KeyK', 4)).toBe(72)
    expect(noteForKey('KeyA', 0)).toBe(12)
    expect(noteForKey('KeyQ', 4)).toBeNull()
    expect(noteForKey('Semicolon', 9)).toBeNull() // above 127
  })

  it('shifts and clamps octaves', () => {
    expect(octaveStep('KeyZ')).toBe(-1)
    expect(octaveStep('KeyX')).toBe(1)
    expect(octaveStep('KeyA')).toBe(0)
    expect(clampOctave(-2)).toBe(0)
    expect(clampOctave(12)).toBe(8)
  })

  it('ignores repeats, shortcuts and typing in fields', () => {
    const base = { repeat: false, ctrlKey: false, metaKey: false, altKey: false, target: document.body }
    expect(ignoreKeyEvent(base)).toBe(false)
    expect(ignoreKeyEvent({ ...base, repeat: true })).toBe(true)
    expect(ignoreKeyEvent({ ...base, ctrlKey: true })).toBe(true)
    const input = document.createElement('input')
    document.body.append(input)
    expect(ignoreKeyEvent({ ...base, target: input })).toBe(true)
    const field = document.createElement('div')
    field.innerHTML = '<label><input type="text"></label><textarea></textarea>'
    expect(ignoreKeyEvent({ ...base, target: field.querySelector('input') })).toBe(true)
    expect(ignoreKeyEvent({ ...base, target: field.querySelector('textarea') })).toBe(true)
  })

  it('keeps playing while a knob, button, radio or checkbox has the focus', () => {
    const base = { repeat: false, ctrlKey: false, metaKey: false, altKey: false }
    const box = document.createElement('div')
    box.innerHTML = '<input type="range"><input type="radio"><input type="checkbox"><button>b</button>'
    for (const el of box.children) expect(ignoreKeyEvent({ ...base, target: el }), el.outerHTML).toBe(false)
  })
})

describe('history shortcuts', () => {
  const base = { key: 'z', ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, target: document.body }

  it('maps Ctrl or Cmd + Z to undo, Shift + Z and Ctrl + Y to redo', () => {
    expect(historyShortcut(base)).toBe('undo')
    expect(historyShortcut({ ...base, ctrlKey: false, metaKey: true })).toBe('undo')
    expect(historyShortcut({ ...base, key: 'Z', shiftKey: true })).toBe('redo')
    expect(historyShortcut({ ...base, key: 'y' })).toBe('redo')
  })

  it('leaves the other keys, a plain Z and text fields alone', () => {
    expect(historyShortcut({ ...base, ctrlKey: false })).toBeNull()
    expect(historyShortcut({ ...base, key: 'a' })).toBeNull()
    expect(historyShortcut({ ...base, altKey: true })).toBeNull()
    expect(historyShortcut({ ...base, key: 'y', shiftKey: true })).toBeNull()
    const input = document.createElement('input')
    input.type = 'text'
    document.body.append(input)
    expect(historyShortcut({ ...base, target: input })).toBeNull()
    const range = document.createElement('input')
    range.type = 'range'
    expect(historyShortcut({ ...base, target: range })).toBe('undo')
  })
})
