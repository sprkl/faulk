import { describe, expect, it } from 'vitest'
import { DEVICES } from './generated/protocol'
import { formatValue, type ParamDescriptor } from './devices'
import { formatGain, formatPan } from './project/tracks'
import { parseGain, parsePan, parseParam, parseQuantity } from './value-entry'

const param = (over: Record<string, unknown>) => ({ id: 0, key: 'p', description: '', unit: 'none', min: 0, max: 1, default: 0, scale: 'linear', step: 0, smoothing_seconds: 0, values: [], ...over }) as unknown as ParamDescriptor

const seconds = param({ unit: 'seconds', min: 0.002, max: 30, scale: 'log' })
const hertz = param({ unit: 'hertz', min: 30, max: 12000, scale: 'log' })
const gain = param({ unit: 'gain', min: 0, max: 1 })
const level = param({ unit: 'none', min: 0, max: 1 })
const ratio = param({ unit: 'none', min: 1, max: 20, scale: 'log' })
const steps = param({ unit: 'semitones', min: -12, max: 12, step: 1 })

describe('parseQuantity', () => {
  it('reads a number and its unit, with a comma, a true minus or no space', () => {
    expect(parseQuantity('250 ms')).toEqual({ value: 250, unit: 'ms' })
    expect(parseQuantity('250ms')).toEqual({ value: 250, unit: 'ms' })
    expect(parseQuantity('−6,5 dB')).toEqual({ value: -6.5, unit: 'db' })
    expect(parseQuantity('1.2K')).toEqual({ value: 1.2, unit: 'k' })
    expect(parseQuantity('40 %')).toEqual({ value: 40, unit: '%' })
  })
  it('refuses anything else', () => {
    for (const bad of ['', 'abc', '1 2', '1..2', '--1', 'ms', '12 ms 4', 'NaN']) expect(parseQuantity(bad), bad).toBeNull()
  })
})

describe('parseParam', () => {
  it('seconds: ms and s, a bare number in the unit shown', () => {
    expect(parseParam(seconds, '300 ms', 0.005)).toBe(0.3)
    expect(parseParam(seconds, '300', 0.005)).toBe(0.3)
    expect(parseParam(seconds, '1.5 s', 0.005)).toBe(1.5)
    expect(parseParam(seconds, '2', 1.5)).toBe(2)
    expect(parseParam(seconds, '2.5 hz', 0.5)).toBeNull()
  })
  it('hertz: Hz, kHz and k', () => {
    expect(parseParam(hertz, '440 Hz', 52)).toBe(440)
    expect(parseParam(hertz, '1.2k', 52)).toBe(1200)
    expect(parseParam(hertz, '1,5 kHz', 52)).toBe(1500)
    expect(parseParam(hertz, '2', 5000)).toBe(2000)
    expect(parseParam(hertz, '2 ms', 52)).toBeNull()
  })
  it('gain: dB, silence and the bound', () => {
    expect(parseParam(gain, '-6 dB', 0.8)).toBeCloseTo(0.501187, 5)
    expect(parseParam(gain, '−∞ dB', 0.8)).toBe(0)
    expect(parseParam(gain, '-inf', 0.8)).toBe(0)
    expect(parseParam(gain, '12 dB', 0.8)).toBe(1)
  })
  it('a level from 0 to 1 is a percentage; another plain number is a number', () => {
    expect(parseParam(level, '35', 0.3)).toBe(0.35)
    expect(parseParam(level, '35 %', 0.3)).toBe(0.35)
    expect(parseParam(level, '250 %', 0.3)).toBe(1)
    expect(parseParam(level, '3 ms', 0.3)).toBeNull()
    expect(parseParam(ratio, '4', 2)).toBe(4)
    expect(parseParam(ratio, '40', 2)).toBe(20)
    expect(parseParam(ratio, '4 %', 2)).toBeNull()
  })
  it('clamps to the bounds and rounds to the step', () => {
    expect(parseParam(seconds, '1 ms', 0.1)).toBe(0.002)
    expect(parseParam(seconds, '99 s', 0.1)).toBe(30)
    expect(parseParam(steps, '3.4 st', 0)).toBe(3)
    expect(parseParam(steps, '-40', 0)).toBe(-12)
  })
  it('takes back every readout of every parameter of the catalogue', () => {
    for (const device of DEVICES) {
      for (const p of device.params as readonly ParamDescriptor[]) {
        if (p.values.length > 0) continue
        for (const value of [p.min, p.default, p.max]) {
          const shown = formatValue(p, value)
          const back = parseParam(p, shown, value)
          expect(back, `${device.kind}.${p.key} ${shown}`).not.toBeNull()
          // The readout is rounded; reading it back lands within what it shows, inside the bounds.
          expect(back!).toBeGreaterThanOrEqual(p.min)
          expect(back!).toBeLessThanOrEqual(p.max)
        }
      }
    }
  })
})

describe('parseGain', () => {
  it('reads dB, rounds to 0.1 and clamps to the fader', () => {
    expect(parseGain('-6')).toBe(-6)
    expect(parseGain('-6.04 dB')).toBe(-6)
    expect(parseGain('0')).toBe(0)
    expect(parseGain('-0.01')).toBe(0)
    expect(parseGain('+20')).toBe(6)
    expect(parseGain('-200')).toBeNull()
  })
  it('reads silence, and takes back what the readout shows', () => {
    expect(parseGain('−∞ dB')).toBeNull()
    expect(parseGain('-inf')).toBeNull()
    for (const db of [null, 0, -6.3, 6, -96 + 0.1]) expect(parseGain(`${formatGain(db)} dB`), String(db)).toBe(db)
  })
  it('refuses a foreign unit and text', () => {
    for (const bad of ['', 'loud', '3 ms', '1 2']) expect(parseGain(bad), bad).toBeUndefined()
  })
})

describe('parsePan', () => {
  it('reads the centre, a side and a signed percentage', () => {
    expect(parsePan('C')).toBe(0)
    expect(parsePan('L30')).toBe(-0.3)
    expect(parsePan('r25')).toBe(0.25)
    expect(parsePan('G 10 %')).toBe(-0.1)
    expect(parsePan('D10')).toBe(0.1)
    expect(parsePan('-30')).toBe(-0.3)
    expect(parsePan('40 %')).toBe(0.4)
    expect(parsePan('250')).toBe(1)
    expect(parsePan('R300')).toBe(1)
  })
  it('takes back what the readout shows', () => {
    for (const pan of [-1, -0.33, 0, 0.5, 1]) expect(parsePan(formatPan(pan))).toBe(Math.round(pan * 100) / 100)
  })
  it('refuses a foreign unit and text', () => {
    for (const bad of ['', 'left', '3 ms', 'L', 'Lx']) expect(parsePan(bad), bad).toBeUndefined()
  })
})
