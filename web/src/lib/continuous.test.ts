import { describe, expect, it } from 'vitest'
import { createDrag, DRAG_SLOP_PX, FINE_RATIO, fineKey, keyAction, KEY_STEP, MAGNET_MAX_SHARE, MAGNET_PX } from './continuous'

// A control 200 px long whose default value sits in the middle. The magnet is a plateau of
// 2 * MAGNET_PX pixels of travel, from the moment the value reaches the default.
const TRAVEL = 200
const HOME = 0.5
const px = (position: number) => position * TRAVEL

describe('the magnet', () => {
  it('holds the default value for 2 * MAGNET_PX of travel, from below and from above', () => {
    const up = createDrag({ position: 0.3, home: HOME, travel: TRAVEL })
    expect(up.move(px(0.2) - 1, false)).toBeLessThan(HOME)
    expect(up.move(px(0.2), false)).toBe(HOME)
    expect(up.move(px(0.2) + MAGNET_PX, false)).toBe(HOME)
    expect(up.move(px(0.2) + 2 * MAGNET_PX, false)).toBe(HOME)
    expect(up.move(px(0.2) + 2 * MAGNET_PX + 1, false)).toBeGreaterThan(HOME)
    const down = createDrag({ position: 0.7, home: HOME, travel: TRAVEL })
    expect(down.move(-px(0.2) + 1, false)).toBeGreaterThan(HOME)
    expect(down.move(-px(0.2), false)).toBe(HOME)
    expect(down.move(-px(0.2) - 2 * MAGNET_PX, false)).toBe(HOME)
    expect(down.move(-px(0.2) - 2 * MAGNET_PX - 1, false)).toBeLessThan(HOME)
  })

  it('uses up pointer travel instead of skipping values: no jump, no gap', () => {
    const drag = createDrag({ position: 0.3, home: HOME, travel: TRAVEL })
    const past = drag.move(px(0.2) + 2 * MAGNET_PX + 1, false)
    expect(past).toBeCloseTo(HOME + 1 / TRAVEL, 10)
    // One pixel of pointer never moves the value by more than one pixel's worth.
    let last = 0.3
    for (let step = 0; step <= px(0.8) + 1; step++) {
      const value = createDrag({ position: 0.3, home: HOME, travel: TRAVEL }).move(step, false)
      expect(value).toBeGreaterThanOrEqual(last)
      expect(value - last).toBeLessThanOrEqual(1 / TRAVEL + 1e-9)
      last = value
    }
    expect(last).toBe(1)
  })

  it('starts from the default value with the plateau on both sides, and from beside it without a jump', () => {
    const at = createDrag({ position: HOME, home: HOME, travel: TRAVEL })
    expect(at.move(MAGNET_PX, false)).toBe(HOME)
    expect(at.move(-MAGNET_PX, false)).toBe(HOME)
    expect(at.move(MAGNET_PX + 10, false)).toBeCloseTo(HOME + 10 / TRAVEL, 10)
    const beside = createDrag({ position: HOME + 0.2, home: HOME, travel: TRAVEL })
    expect(beside.move(0, false)).toBeCloseTo(HOME + 0.2, 10)
    expect(beside.move(10, false)).toBeCloseTo(HOME + 0.2 + 10 / TRAVEL, 10)
  })

  it('is smaller on a short control, and absent without a default', () => {
    const short = 40
    const zone = MAGNET_MAX_SHARE * short
    expect(zone).toBeLessThan(MAGNET_PX)
    const drag = createDrag({ position: HOME, home: HOME, travel: short })
    expect(drag.move(zone, false)).toBe(HOME)
    expect(drag.move(zone + 1, false)).toBeGreaterThan(HOME)
    const free = createDrag({ position: 0.2, home: null, travel: TRAVEL })
    expect(free.move(px(0.3), false)).toBeCloseTo(0.5, 10)
    expect(free.move(px(0.3) + 2, false)).toBeCloseTo(0.5 + 2 / TRAVEL, 10)
  })

  it('clamps at the ends and comes back as soon as the pointer does', () => {
    const drag = createDrag({ position: 0.9, home: HOME, travel: TRAVEL })
    expect(drag.move(5000, false)).toBe(1)
    expect(drag.move(-5000, false)).toBe(0)
    const back = createDrag({ position: 0.9, home: HOME, travel: TRAVEL })
    expect(back.move(5000, false)).toBe(1)
    // A few pixels back from the end, not the whole overshoot.
    expect(back.move(5000 - 10, false)).toBeCloseTo(1 - 10 / TRAVEL, 10)
    expect(back.move(NaN, false)).toBeCloseTo(1 - 10 / TRAVEL, 10)
  })

  it('reaches both ends when the default is at one of them', () => {
    const low = createDrag({ position: 0.5, home: 0, travel: TRAVEL })
    expect(low.move(-5000, false)).toBe(0)
    expect(low.move(-5000 + 20, false)).toBeGreaterThan(0)
    expect(createDrag({ position: 0.5, home: 1, travel: TRAVEL }).move(5000, false)).toBe(1)
  })
})

describe('the fine mode', () => {
  it('moves FINE_RATIO times less per pixel, and has no magnet', () => {
    const drag = createDrag({ position: 0.3, home: HOME, travel: TRAVEL })
    drag.move(0, true)
    expect(drag.move(20, true)).toBeCloseTo(0.3 + 20 / TRAVEL / FINE_RATIO, 10)
    // Going through the default value: the fine mode does not stop on it.
    const through = createDrag({ position: HOME - 0.0033, home: HOME, travel: TRAVEL })
    through.move(0, true)
    const seen = Array.from({ length: 12 }, (_, step) => through.move(step, true))
    expect(seen).not.toContain(HOME)
    expect(new Set(seen).size).toBe(seen.length)
    expect(seen[0]! < HOME && seen.at(-1)! > HOME).toBe(true)
  })

  it('does not jump when the key is pressed or released in the middle of a drag', () => {
    const drag = createDrag({ position: 0.2, home: null, travel: TRAVEL })
    const before = drag.move(20, false)
    expect(drag.move(20, true)).toBeCloseTo(before, 10)
    const fine = drag.move(40, true)
    expect(fine).toBeCloseTo(before + 20 / TRAVEL / FINE_RATIO, 10)
    expect(drag.move(40, false)).toBeCloseTo(fine, 10)
    expect(drag.move(50, false)).toBeCloseTo(fine + 10 / TRAVEL, 10)
  })

  it('does not jump either when leaving the fine mode beside the default value', () => {
    const drag = createDrag({ position: 0.3, home: HOME, travel: TRAVEL })
    drag.move(0, true)
    const fine = drag.move(10, true)
    expect(drag.move(10, false)).toBeCloseTo(fine, 10)
  })

  it('reads Cmd on macOS and Ctrl elsewhere', () => {
    expect(fineKey({ ctrlKey: true, metaKey: false }, false)).toBe(true)
    expect(fineKey({ ctrlKey: false, metaKey: true }, false)).toBe(false)
    expect(fineKey({ ctrlKey: false, metaKey: true }, true)).toBe(true)
    expect(fineKey({ ctrlKey: true, metaKey: false }, true)).toBe(false)
  })
})

describe('the keyboard', () => {
  const at = (key: string, position: number, fine = false, home: number | null = HOME) => keyAction(key, { position, home, fine })

  it('steps with the arrows, finer with the fine key, and clamps', () => {
    expect(at('ArrowUp', 0.1)).toEqual({ position: expect.closeTo(0.1 + KEY_STEP, 10) })
    expect(at('ArrowRight', 0.1)).toEqual({ position: expect.closeTo(0.1 + KEY_STEP, 10) })
    expect(at('ArrowDown', 0.1)).toEqual({ position: expect.closeTo(0.1 - KEY_STEP, 10) })
    expect(at('ArrowLeft', 0.1, true)).toEqual({ position: expect.closeTo(0.1 - KEY_STEP / FINE_RATIO, 10) })
    expect(at('ArrowDown', 0)).toEqual({ position: 0 })
    expect(at('ArrowUp', 1)).toEqual({ position: 1 })
    expect(at('PageUp', 0.2, false, null)).toEqual({ position: expect.closeTo(0.3, 10) })
    expect(at('PageDown', 0.05)).toEqual({ position: 0 })
    expect(at('Home', 0.4)).toEqual({ position: 0 })
    expect(at('End', 0.4)).toEqual({ position: 1 })
  })

  it('stops on the default value instead of stepping over it, except in the fine mode', () => {
    expect(at('ArrowUp', HOME - KEY_STEP / 2)).toEqual({ position: HOME })
    expect(at('ArrowDown', HOME + KEY_STEP / 2)).toEqual({ position: HOME })
    expect(at('PageUp', HOME - 0.01)).toEqual({ position: HOME })
    // From the default value it moves on.
    expect(at('ArrowUp', HOME)).toEqual({ position: expect.closeTo(HOME + KEY_STEP, 10) })
    expect(at('ArrowUp', HOME - KEY_STEP / 20, true)).toEqual({ position: expect.closeTo(HOME - KEY_STEP / 20 + KEY_STEP / FINE_RATIO, 10) })
  })

  it('resets with Delete and Backspace, and ignores every other key (a note key above all)', () => {
    expect(at('Delete', 0.2)).toEqual({ reset: true })
    expect(at('Backspace', 0.2)).toEqual({ reset: true })
    for (const key of ['a', 'KeyA', 'k', ';', ' ', 'Enter', 'Escape', 'F2', 'Tab', 'z', 'x']) expect(at(key, 0.2), key).toBeNull()
  })
})

describe('the slop', () => {
  it('is a few pixels, so that a click or a double click never moves a control', () => {
    expect(DRAG_SLOP_PX).toBeGreaterThanOrEqual(2)
    expect(DRAG_SLOP_PX).toBeLessThanOrEqual(5)
  })
})
