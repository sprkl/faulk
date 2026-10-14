import { describe, expect, it } from 'vitest'
import { DEFAULT_PARAMS, KNOB_PARAMS, TIME_MAX, TIME_MIN, formatValue, positionOf, valueAt } from './params'

describe('knob scaling', () => {
  it('round-trips every parameter', () => {
    for (const param of KNOB_PARAMS) {
      for (const position of [0, 0.25, 0.5, 0.75, 1]) {
        expect(positionOf(param, valueAt(param, position))).toBeCloseTo(position, 6)
      }
    }
  })

  it('maps time knobs logarithmically between their bounds', () => {
    expect(valueAt('attack', 0)).toBeCloseTo(TIME_MIN)
    expect(valueAt('release', 1)).toBeCloseTo(TIME_MAX)
    expect(valueAt('decay', 0.5)).toBeCloseTo(Math.sqrt(TIME_MIN * TIME_MAX))
    expect(positionOf('attack', 100)).toBe(1)
    expect(positionOf('attack', 0)).toBe(0)
  })

  it('formats values with units', () => {
    expect(formatValue('attack', 0.005)).toBe('5 ms')
    expect(formatValue('release', 2.5)).toBe('2.50 s')
    expect(formatValue('sustain', 0.7)).toBe('70 %')
    expect(formatValue('gain', 1)).toBe('0.0 dB')
    expect(formatValue('gain', 0)).toBe('−∞ dB')
  })

  it('places the engine defaults inside the knob ranges', () => {
    for (const param of KNOB_PARAMS) {
      const p = positionOf(param, DEFAULT_PARAMS[param])
      expect(p).toBeGreaterThan(0)
      expect(p).toBeLessThan(1)
    }
  })
})
