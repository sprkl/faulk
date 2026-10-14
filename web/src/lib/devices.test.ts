import { describe, expect, it } from 'vitest'
import { DEVICES } from './generated/protocol'
import { choiceParams, choices, formatValue, knobParams, labelKey, positionOf, SYNTH, valueAt } from './devices'
import { messages } from './i18n/app'
import { DEFAULT_PARAMS } from './params'

describe('device panels from the descriptors', () => {
  it('builds the synth panel from its descriptor', () => {
    expect(SYNTH.name).toBe('Oscillator')
    expect(knobParams(SYNTH).map((p) => p.key)).toEqual(['attack', 'decay', 'sustain', 'release', 'gain'])
    expect(choiceParams(SYNTH).map((p) => p.key)).toEqual(['waveform'])
    expect(choices(choiceParams(SYNTH)[0])).toEqual([
      { value: 0, key: 'sine' },
      { value: 1, key: 'saw' },
      { value: 2, key: 'square' },
      { value: 3, key: 'triangle' },
    ])
  })

  it('round-trips every knob between its bounds', () => {
    for (const d of DEVICES) {
      for (const p of knobParams(d)) {
        for (const position of [0, 0.25, 0.5, 0.75, 1]) {
          expect(positionOf(p, valueAt(p, position)), p.key).toBeCloseTo(position, 6)
        }
        expect(valueAt(p, 0)).toBeCloseTo(p.min)
        expect(valueAt(p, 1)).toBeCloseTo(p.max)
        expect(positionOf(p, p.max * 10 + 10)).toBe(1)
        expect(positionOf(p, p.min - 10)).toBe(0)
      }
    }
  })

  it('maps a log parameter logarithmically', () => {
    const attack = SYNTH.params.find((p) => p.scale === 'log')!
    expect(valueAt(attack, 0.5)).toBeCloseTo(Math.sqrt(attack.min * attack.max))
  })

  it('formats values by unit', () => {
    const [attack, sustain] = [knobParams(SYNTH)[0], knobParams(SYNTH)[2]]
    expect(formatValue(attack, 0.005)).toBe('5 ms')
    expect(formatValue(attack, 2.5)).toBe('2.50 s')
    expect(formatValue(sustain, 0.7)).toBe('70 %')
  })

  it('places the engine defaults inside the knob ranges', () => {
    for (const p of knobParams(SYNTH)) {
      expect(p.default, p.key).toBe(DEFAULT_PARAMS[p.key])
      const at = positionOf(p, p.default)
      expect(at).toBeGreaterThan(0)
      expect(at).toBeLessThan(1)
    }
  })

  it('has a label in every language for every parameter and value of every device', () => {
    for (const d of DEVICES) {
      const keys = d.params.flatMap((p) => [p.key, ...p.values]).map((k) => labelKey(d, k))
      for (const lang of ['en', 'fr'] as const) {
        const catalogue = messages[lang] as Record<string, string>
        for (const key of keys) expect(catalogue[key]?.trim(), `${lang} ${key}`).toBeTruthy()
      }
    }
  })
})
