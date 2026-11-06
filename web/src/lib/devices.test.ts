import { describe, expect, it } from 'vitest'
import { DEVICES, PARAM_IDS } from './generated/protocol'
import { choiceParams, choices, descriptorOf, formatValue, knobParams, labelKey, positionOf, SHOWN_IN_MODE, shownParams, SYNTH, valueAt } from './devices'
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
    const gain = knobParams(SYNTH)[4]
    expect(gain.unit).toBe('gain')
    expect(formatValue(gain, 1)).toBe('0.0 dB')
    expect(formatValue(gain, 0.8)).toBe('-1.9 dB')
    expect(formatValue(gain, 0)).toBe('−∞ dB')
    // A plain level that stops short of 1 (the echo's feedback) is a percentage too.
    const feedback = descriptorOf('faulk.echo').params.find((p) => p.key === 'feedback')!
    expect(formatValue(feedback, 0.4)).toBe('40 %')
  })

  it('places the engine defaults inside the knob ranges', () => {
    for (const p of knobParams(SYNTH)) {
      expect(p.default, p.key).toBe(DEFAULT_PARAMS[p.key])
      const at = positionOf(p, p.default)
      expect(at).toBeGreaterThan(0)
      expect(at).toBeLessThan(1)
    }
  })

  it('can set every parameter of the synth through device.set_param', () => {
    for (const p of SYNTH.params) expect(Object.keys(PARAM_IDS), p.key).toContain(p.key)
    expect(Object.keys(PARAM_IDS).length).toBe(SYNTH.params.length)
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

describe('parameters shown in one mode only (#472)', () => {
  it('names only devices, parameters and values that the descriptors have', () => {
    for (const [kind, rules] of Object.entries(SHOWN_IN_MODE)) {
      const device = descriptorOf(kind)
      for (const [key, rule] of Object.entries(rules)) {
        expect(
          device.params.map((p) => p.key),
          `${kind}.${key}`,
        ).toContain(key)
        const by = device.params.find((p) => p.key === rule.by)
        expect(by, `${kind}.${key} is decided by ${rule.by}`).toBeDefined()
        for (const value of rule.values) expect(by!.values as readonly string[], `${kind}.${key}`).toContain(value)
      }
    }
  })

  it('shows the Echo delay time when free and its note value when synced, whatever the stored value of the other', () => {
    const echo = descriptorOf('faulk.echo')
    const mode = choices(echo.params.find((p) => p.key === 'mode')!)
    const keys = (value: number | undefined) => shownParams(echo, (key) => (key === 'mode' ? value : undefined)).map((p) => p.key)
    const synced = mode.find((c) => c.key === 'synced')!.value
    const free = mode.find((c) => c.key === 'free')!.value
    expect(keys(synced)).toEqual(['mode', 'division', 'feedback', 'mix', 'time_change'])
    expect(keys(free)).toEqual(['mode', 'time', 'feedback', 'mix', 'time_change'])
    // Without a stored value the descriptor's default decides (Echo starts synced).
    expect(keys(undefined)).toEqual(keys(synced))
    // A device with no rule shows everything.
    const drums = descriptorOf('faulk.drums')
    expect(shownParams(drums, () => undefined)).toEqual(drums.params)
  })

  it('shows a Stack voice’s interval, velocity and chance only while the voice is on (#690)', () => {
    const stack = descriptorOf('faulk.stack')
    // By default voices 1 and 2 are on, 3 to 6 off.
    const keys = shownParams(stack, () => undefined).map((p) => p.key)
    expect(keys).toContain('voice2_interval')
    expect(keys).toContain('voice3')
    expect(keys).not.toContain('voice3_interval')
    expect(keys).not.toContain('voice6_chance')
    const on = shownParams(stack, (key) => (key === 'voice3' ? 1 : undefined)).map((p) => p.key)
    expect(on).toEqual(expect.arrayContaining(['voice3_interval', 'voice3_velocity', 'voice3_chance']))
    expect(stack.role).toBe('midi_effect')
    expect(stack.name).toBe('Stack')
  })
})
