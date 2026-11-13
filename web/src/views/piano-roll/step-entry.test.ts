import { describe, expect, it } from 'vitest'
import type { Note } from '../../lib/generated/protocol'
import { PLAYING_KEYS, BARE_SHORTCUT_CODES, QUANTIZE_CODE } from '../../lib/keyboard'
import {
  advanceTo,
  barBeat,
  dropLast,
  isStepEntryToggle,
  LENGTHEN_CODE,
  lengthenLast,
  REST_CODE,
  STEP_ENTRY_CODE,
  stepEntryKey,
  type EntryStep,
} from './step-entry'

const key = (over: Partial<Parameters<typeof stepEntryKey>[0]> = {}) => ({
  code: 'KeyA',
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  repeat: false,
  ...over,
})

const note = (over: Partial<Note> = {}): Note => ({ start: 0, pitch: 60, length: 240, velocity: 100, ...over })

describe('stepEntryKey', () => {
  it('takes every musical-typing letter as a note', () => {
    for (const code of PLAYING_KEYS) expect(stepEntryKey(key({ code }))).toEqual({ kind: 'note', code })
  })

  it('maps rest, lengthen and Backspace', () => {
    expect(stepEntryKey(key({ code: REST_CODE }))).toEqual({ kind: 'rest' })
    expect(stepEntryKey(key({ code: LENGTHEN_CODE }))).toEqual({ kind: 'lengthen' })
    expect(stepEntryKey(key({ code: 'Backspace' }))).toEqual({ kind: 'back' })
  })

  it('leaves Ctrl, Cmd, Alt and repeats to others', () => {
    for (const over of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { repeat: true }]) {
      expect(stepEntryKey(key(over))).toBeNull()
      expect(stepEntryKey(key({ code: REST_CODE, ...over }))).toBeNull()
      expect(stepEntryKey(key({ code: 'Backspace', ...over }))).toBeNull()
    }
  })

  it('ignores Shift on the three control keys and every other key', () => {
    for (const code of [REST_CODE, LENGTHEN_CODE, 'Backspace']) expect(stepEntryKey(key({ code, shiftKey: true }))).toBeNull()
    for (const code of ['Space', 'Enter', 'Tab', 'Home', 'ArrowLeft', 'Digit1', 'KeyZ', 'KeyX', 'KeyQ', 'Backquote'])
      expect(stepEntryKey(key({ code }))).toBeNull()
  })

  it('uses keys that are neither played nor bare shortcuts already, so no key does two things', () => {
    expect(PLAYING_KEYS).not.toContain(REST_CODE)
    expect(PLAYING_KEYS).not.toContain(LENGTHEN_CODE)
    expect(BARE_SHORTCUT_CODES).not.toContain(REST_CODE)
    expect(BARE_SHORTCUT_CODES).not.toContain(LENGTHEN_CODE)
    expect([REST_CODE, LENGTHEN_CODE, QUANTIZE_CODE]).toEqual(['Period', 'Comma', 'KeyQ'])
  })
})

describe('isStepEntryToggle', () => {
  it('is Alt+E alone, without a repeat', () => {
    expect(isStepEntryToggle(key({ code: STEP_ENTRY_CODE, altKey: true }))).toBe(true)
    expect(isStepEntryToggle(key({ code: STEP_ENTRY_CODE }))).toBe(false)
    expect(isStepEntryToggle(key({ code: STEP_ENTRY_CODE, altKey: true, shiftKey: true }))).toBe(false)
    expect(isStepEntryToggle(key({ code: STEP_ENTRY_CODE, altKey: true, ctrlKey: true }))).toBe(false)
    expect(isStepEntryToggle(key({ code: STEP_ENTRY_CODE, altKey: true, repeat: true }))).toBe(false)
    expect(isStepEntryToggle(key({ code: 'KeyG', altKey: true }))).toBe(false)
  })
})

describe('advanceTo', () => {
  it('moves on while the clip has room and says null at its end', () => {
    expect(advanceTo(240, 1920)).toBe(240)
    expect(advanceTo(1919, 1920)).toBe(1919)
    expect(advanceTo(1920, 1920)).toBeNull()
    expect(advanceTo(2400, 1920)).toBeNull()
  })
})

describe('lengthenLast', () => {
  it('is null before any step', () => {
    expect(lengthenLast([], 240, 1920)).toBeNull()
  })

  it('grows the last note by one unit and puts the cursor at its end', () => {
    const n = note()
    const steps: EntryStep[] = [{ start: 0, length: 240, notes: [n] }]
    const r = lengthenLast(steps, 120, 1920)!
    expect(r.changes).toEqual([{ from: n, to: { ...n, length: 360 } }])
    expect(r.steps).toEqual([{ start: 0, length: 360, notes: [{ ...n, length: 360 }] }])
    expect(r.cursor).toBe(360)
    expect(r.cut).toBe(false)
  })

  it('touches only the last step, and every note of a chord', () => {
    const a = note({ pitch: 60 })
    const b = note({ pitch: 64 })
    const first = { start: 0, length: 240, notes: [note({ pitch: 50 })] }
    const r = lengthenLast(
      [
        first,
        {
          start: 240,
          length: 240,
          notes: [
            { ...a, start: 240 },
            { ...b, start: 240 },
          ],
        },
      ],
      240,
      1920,
    )!
    expect(r.steps[0]).toBe(first)
    expect(r.changes.map((c) => c.to.length)).toEqual([480, 480])
    expect(r.cursor).toBe(720)
  })

  it('stops at the clip end and says it was cut', () => {
    const n = note({ start: 1800, length: 60 })
    const r = lengthenLast([{ start: 1800, length: 60, notes: [n] }], 240, 1920)!
    expect(r.changes[0]!.to.length).toBe(120)
    expect(r.cut).toBe(true)
    expect(r.cursor).toBe(1920)
    const full = lengthenLast([{ start: 1800, length: 120, notes: [{ ...n, length: 120 }] }], 240, 1920)!
    expect(full.changes).toEqual([])
    expect(full.steps.at(-1)!.length).toBe(120)
  })

  it('lengthens a rest too: more silence, no note to update', () => {
    const r = lengthenLast([{ start: 480, length: 240, notes: [] }], 240, 1920)!
    expect(r.changes).toEqual([])
    expect(r.steps).toEqual([{ start: 480, length: 480, notes: [] }])
    expect(r.cursor).toBe(960)
  })
})

describe('dropLast', () => {
  const n = note()
  it('is null before any step', () => {
    expect(dropLast([], () => true)).toBeNull()
  })

  it('removes the last step and puts the cursor back where it began', () => {
    const steps: EntryStep[] = [
      { start: 0, length: 240, notes: [note()] },
      { start: 240, length: 240, notes: [{ ...n, start: 240 }] },
    ]
    const r = dropLast(steps, () => true)!
    expect(r.steps).toEqual([steps[0]])
    expect(r.notes).toEqual([{ ...n, start: 240 }])
    expect(r.cursor).toBe(240)
  })

  it('keeps going back through a rest, with no note to remove', () => {
    const r = dropLast([{ start: 0, length: 240, notes: [] }], () => true)!
    expect(r.notes).toEqual([])
    expect(r.cursor).toBe(0)
    expect(r.steps).toEqual([])
  })

  it('leaves out notes an undo already took', () => {
    const r = dropLast([{ start: 0, length: 240, notes: [n] }], () => false)!
    expect(r.notes).toEqual([])
    expect(r.cursor).toBe(0)
  })
})

describe('barBeat', () => {
  it('counts from 1, the beat to a hundredth', () => {
    expect(barBeat(0, 480, 4)).toEqual({ bar: 1, beat: 1 })
    expect(barBeat(720, 480, 4)).toEqual({ bar: 1, beat: 2.5 })
    expect(barBeat(1920, 480, 4)).toEqual({ bar: 2, beat: 1 })
  })
})
