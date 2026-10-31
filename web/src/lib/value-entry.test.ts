import { describe, expect, it } from 'vitest'
import { DEVICES } from './generated/protocol'
import { formatValue, type ParamDescriptor } from './devices'
import { formatGain, formatPan } from './project/tracks'
import { baseUnit, parseGain, parseGainEntry, parsePan, parsePanEntry, parseParam, parseParamEntry, parseQuantity } from './value-entry'

const param = (over: Record<string, unknown>) =>
  ({
    id: 0,
    key: 'p',
    description: '',
    unit: 'none',
    min: 0,
    max: 1,
    default: 0,
    scale: 'linear',
    step: 0,
    smoothing_seconds: 0,
    values: [],
    ...over,
  }) as unknown as ParamDescriptor

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
  it('seconds: ms and s, a bare number in ms whatever the readout shows', () => {
    expect(parseParam(seconds, '300 ms')).toBe(0.3)
    expect(parseParam(seconds, '300')).toBe(0.3)
    expect(parseParam(seconds, '1.5 s')).toBe(1.5)
    expect(parseParam(seconds, '2')).toBe(0.002)
    expect(parseParam(seconds, '1200')).toBe(1.2)
    expect(parseParam(seconds, '1,2 s')).toBe(1.2)
    expect(parseParam(seconds, '2.5 hz')).toBeNull()
  })
  it('hertz: Hz, kHz and k', () => {
    expect(parseParam(hertz, '440 Hz')).toBe(440)
    expect(parseParam(hertz, '1.2k')).toBe(1200)
    expect(parseParam(hertz, '1,5 kHz')).toBe(1500)
    expect(parseParam(hertz, '450')).toBe(450)
    expect(parseParam(hertz, '0.5 kHz')).toBe(500)
    expect(parseParam(hertz, '0,5 khz')).toBe(500)
    expect(parseParam(hertz, '15')).toBe(30)
    expect(parseParam(hertz, '2 ms')).toBeNull()
  })
  it('gain: dB, silence and the bound', () => {
    expect(parseParam(gain, '-6 dB')).toBeCloseTo(0.501187, 5)
    expect(parseParam(gain, '−∞ dB')).toBe(0)
    expect(parseParam(gain, '-inf')).toBe(0)
    expect(parseParam(gain, '12 dB')).toBe(1)
  })
  it('a level from 0 to 1 is a percentage; another plain number is a number', () => {
    expect(parseParam(level, '35')).toBe(0.35)
    expect(parseParam(level, '35 %')).toBe(0.35)
    expect(parseParam(level, '250 %')).toBe(1)
    expect(parseParam(level, '3 ms')).toBeNull()
    expect(parseParam(ratio, '4')).toBe(4)
    expect(parseParam(ratio, '40')).toBe(20)
    expect(parseParam(ratio, '4 %')).toBeNull()
  })
  it('clamps to the bounds and rounds to the step', () => {
    expect(parseParam(seconds, '1 ms')).toBe(0.002)
    expect(parseParam(seconds, '99 s')).toBe(30)
    expect(parseParam(steps, '3.4 st')).toBe(3)
    expect(parseParam(steps, '-40')).toBe(-12)
  })
  it('takes back every readout of every parameter of the catalogue', () => {
    for (const device of DEVICES) {
      for (const p of device.params as readonly ParamDescriptor[]) {
        if (p.values.length > 0) continue
        for (const value of [p.min, p.default, p.max]) {
          const shown = formatValue(p, value)
          const back = parseParam(p, shown)
          expect(back, `${device.kind}.${p.key} ${shown}`).not.toBeNull()
          // The readout is rounded; reading it back lands within what it shows, inside the bounds.
          expect(back!).toBeGreaterThanOrEqual(p.min)
          expect(back!).toBeLessThanOrEqual(p.max)
        }
      }
    }
  })
})

describe('parseParamEntry (#410)', () => {
  it('reads a bare number in the base unit, whether the readout shows Hz or kHz', () => {
    // The readout does not enter the reading: the same text gives the same value at 52 Hz and at 5 kHz.
    expect(formatValue(hertz, 5000)).toBe('5.00 kHz')
    expect(formatValue(hertz, 52)).toBe('52 Hz')
    expect(parseParamEntry(hertz, '450')).toEqual({ value: 450 })
    expect(parseParamEntry(hertz, '450,5')).toEqual({ value: 450.5 })
    expect(parseParamEntry(seconds, '450')).toEqual({ value: 0.45 })
  })
  it('takes a typed unit as typed', () => {
    expect(parseParamEntry(hertz, '0.5 kHz')).toEqual({ value: 500 })
    expect(parseParamEntry(hertz, '2k')).toEqual({ value: 2000 })
    expect(parseParamEntry(seconds, '1,2 s')).toEqual({ value: 1.2 })
    expect(parseParamEntry(seconds, '1200 ms')).toEqual({ value: 1.2 })
  })
  it('refuses unreadable text and a foreign unit, with no retry', () => {
    for (const bad of ['', 'abc', '1 2', '450 ms', '1e']) expect(parseParamEntry(hertz, bad), bad).toBeNull()
    expect(parseParamEntry(hertz, '15')).toEqual({ value: 30, limit: 30 })
  })
  it('says the bound it limited to, and nothing when only the step rounded', () => {
    expect(parseParamEntry(hertz, '99 kHz')).toEqual({ value: 12000, limit: 12000 })
    expect(parseParamEntry(hertz, '1')).toEqual({ value: 30, limit: 30 })
    expect(parseParamEntry(seconds, '99 s')).toEqual({ value: 30, limit: 30 })
    expect(parseParamEntry(steps, '3.4 st')).toEqual({ value: 3 })
    expect(parseParamEntry(steps, '-40')).toEqual({ value: -12, limit: -12 })
    expect(parseParamEntry(hertz, '12000')).toEqual({ value: 12000 })
  })
  it('names the unit a bare number is read in', () => {
    expect(baseUnit(hertz)).toBe('Hz')
    expect(baseUnit(seconds)).toBe('ms')
    expect(baseUnit(gain)).toBe('dB')
    expect(baseUnit(level)).toBe('%')
    expect(baseUnit(steps)).toBe('st')
    expect(baseUnit(ratio)).toBe('')
  })
  it('gain and pan report a limit', () => {
    expect(parseGainEntry('+20')).toEqual({ gain: 6, limited: true })
    expect(parseGainEntry('3')).toEqual({ gain: 3, limited: false })
    expect(parseGainEntry('x')).toBeNull()
    expect(parsePanEntry('250')).toEqual({ pan: 1, limited: true })
    expect(parsePanEntry('R30')).toEqual({ pan: 0.3, limited: false })
    expect(parsePanEntry('z')).toBeNull()
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
