import { describe, expect, it } from 'vitest'
import { PLAYING_KEYS } from '../../lib/keyboard'
import { CLIP_ROW_MAX, CLIP_ROW_MIN } from './automation/layout'
import { HEIGHT_STEP, HEIGHT_STEP_BIG, heightKey, volumeKey } from './lane-keys'

const key = (k: string, over: Partial<KeyboardEvent> = {}) =>
  ({ key: k, altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, repeat: false, isComposing: false, ...over }) as KeyboardEvent

describe('typing a volume on a track row (#471)', () => {
  it('opens the field with the key typed for a digit, a sign or a decimal point, on the selected row only', () => {
    for (const k of ['0', '5', '9', '-', '+', '.', ',']) expect(volumeKey(key(k), true), k).toEqual({ open: 'typed', text: k })
    for (const k of ['0', '-', 'Enter']) expect(volumeKey(key(k), false), k).toBeNull()
  })

  it('opens it on the value with Enter, not on a held Enter, and F2 is left to the rename (#625)', () => {
    expect(volumeKey(key('Enter'), true)).toEqual({ open: 'value' })
    expect(volumeKey(key('F2'), true)).toBeNull()
    expect(volumeKey(key('Enter', { repeat: true }), true)).toBeNull()
  })

  it('leaves the letters, the chords and an input method alone', () => {
    for (const k of ['a', 'e', 'Delete', ' ', 'ArrowUp', 'Escape', 'Tab']) expect(volumeKey(key(k), true), k).toBeNull()
    expect(volumeKey(key('5', { ctrlKey: true }), true)).toBeNull()
    expect(volumeKey(key('5', { metaKey: true }), true)).toBeNull()
    expect(volumeKey(key('5', { altKey: true }), true)).toBeNull()
    expect(volumeKey(key('5', { isComposing: true }), true)).toBeNull()
  })

  it('uses no key that plays a note', () => {
    // The playing keys are matched by code (KeyA…, Semicolon); none of them gives a digit, a sign or a point.
    for (const code of PLAYING_KEYS) expect(code).toMatch(/^(Key[A-Z]|Semicolon)$/)
  })
})

describe('the height handle of a track row', () => {
  it('steps with the arrows (Shift for a big step), and keeps to the bounds', () => {
    expect(heightKey(key('ArrowDown'), 44)).toBe(44 + HEIGHT_STEP)
    expect(heightKey(key('ArrowUp'), 44)).toBe(44 - HEIGHT_STEP)
    expect(heightKey(key('ArrowDown', { shiftKey: true }), 44)).toBe(44 + HEIGHT_STEP_BIG)
    expect(heightKey(key('ArrowUp'), CLIP_ROW_MIN)).toBe(CLIP_ROW_MIN)
    expect(heightKey(key('ArrowDown', { shiftKey: true }), CLIP_ROW_MAX)).toBe(CLIP_ROW_MAX)
  })

  it('goes to the bounds with Home and End, and back to the default with Enter', () => {
    expect(heightKey(key('Home'), 90)).toBe(CLIP_ROW_MIN)
    expect(heightKey(key('End'), 90)).toBe(CLIP_ROW_MAX)
    expect(heightKey(key('Enter'), 90)).toBe('default')
  })

  it('ignores the other keys and the chords', () => {
    expect(heightKey(key('ArrowLeft'), 44)).toBeNull()
    expect(heightKey(key('a'), 44)).toBeNull()
    expect(heightKey(key('ArrowDown', { ctrlKey: true }), 44)).toBeNull()
  })
})
