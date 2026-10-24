import { describe, expect, it } from 'vitest'
import { MAX_GAIN_DB, MIN_GAIN_DB } from './tracks'
import { FADER_MARKS, gainAt, positionOf, UNITY_POSITION } from './fader'

describe('the fader taper', () => {
  it('puts silence at the bottom and +6 dB at the top', () => {
    expect(positionOf(null)).toBe(0)
    expect(positionOf(MIN_GAIN_DB)).toBe(0)
    expect(positionOf(MAX_GAIN_DB)).toBe(1)
    expect(gainAt(0)).toBeNull()
    expect(gainAt(1)).toBe(MAX_GAIN_DB)
  })

  it('puts 0 dB at unity, exactly, and never swallows a keyboard step', () => {
    expect(positionOf(0)).toBeCloseTo(UNITY_POSITION, 12)
    expect(gainAt(UNITY_POSITION)).toBe(0)
    expect(gainAt(UNITY_POSITION + 0.0005)).toBe(0)
    expect(gainAt(UNITY_POSITION - 0.0005)).toBe(0)
    expect(Object.is(gainAt(UNITY_POSITION - 0.0001), 0)).toBe(true)
    expect(gainAt(UNITY_POSITION + 0.02)).toBeGreaterThan(0.25)
    // One step of the input (0.005) always changes the gain, and 0.78 is a step.
    expect((UNITY_POSITION / 0.005) % 1).toBeCloseTo(0, 9)
    expect(gainAt(UNITY_POSITION - 0.005)).toBeLessThan(0)
    expect(gainAt(UNITY_POSITION + 0.005)).toBeGreaterThan(0)
  })

  it('is monotone, and positions and gains agree to 0.1 dB over the whole travel', () => {
    let last = -Infinity
    for (let i = 1; i <= 1000; i++) {
      const gain = gainAt(i / 1000)
      if (gain === null) continue
      expect(gain).toBeGreaterThanOrEqual(last)
      last = gain
      expect(Math.abs((gainAt(positionOf(gain)) ?? MIN_GAIN_DB) - gain)).toBeLessThanOrEqual(0.1 + 1e-9)
    }
  })

  it('reaches every mark and stays inside the command bounds', () => {
    for (const db of FADER_MARKS) expect(Math.abs((gainAt(positionOf(db)) ?? NaN) - db)).toBeLessThanOrEqual(0.1)
    for (let i = 0; i <= 1000; i++) {
      const gain = gainAt(i / 1000)
      if (gain !== null) {
        expect(gain).toBeGreaterThanOrEqual(MIN_GAIN_DB)
        expect(gain).toBeLessThanOrEqual(MAX_GAIN_DB)
      }
    }
  })

  it('clamps and refuses hostile values', () => {
    expect(gainAt(5)).toBe(MAX_GAIN_DB)
    expect(gainAt(-1)).toBeNull()
    expect(gainAt(NaN)).toBeNull()
    expect(positionOf(NaN)).toBe(0)
    expect(positionOf(Infinity)).toBe(0)
    expect(positionOf(100)).toBe(1)
    expect(positionOf(undefined)).toBe(0)
  })
})
