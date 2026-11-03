import { describe, expect, it } from 'vitest'
import { barTicks, beatTicks, formatPosition, formatTempo, formatTime, parseBarBeat, parseNumerator, parseTempo, secondsOf, stepTempo } from './time'

const common = { numerator: 4, denominator: 4 }

describe('formatPosition', () => {
  it('starts at 1.1.000', () => expect(formatPosition(0, common)).toBe('1.1.000'))
  it('counts beats and ticks', () => {
    expect(formatPosition(960, common)).toBe('1.2.000')
    expect(formatPosition(960 + 37, common)).toBe('1.2.037')
  })
  it('rolls over to the next bar', () => {
    expect(formatPosition(3840, common)).toBe('2.1.000')
    expect(formatPosition(3839, common)).toBe('1.4.959')
  })
  it('follows the signature: 3/8 has 480-tick beats and 1440-tick bars', () => {
    const sig = { numerator: 3, denominator: 8 }
    expect(beatTicks(sig)).toBe(480)
    expect(barTicks(sig)).toBe(1440)
    expect(formatPosition(480, sig)).toBe('1.2.000')
    expect(formatPosition(1440, sig)).toBe('2.1.000')
  })
  it('puts a count-in position in bar 0 and below', () => {
    expect(formatPosition(-1, common)).toBe('0.4.959')
    expect(formatPosition(-3840, common)).toBe('0.1.000')
    expect(formatPosition(-3841, common)).toBe('-1.4.959')
  })
})

describe('formatTime', () => {
  it('reads minutes, seconds and milliseconds', () => {
    expect(formatTime(0)).toBe('00:00.000')
    expect(formatTime(61.5)).toBe('01:01.500')
    expect(formatTime(3599.9996)).toBe('60:00.000')
  })
  it('uses a true minus before the start, none for zero', () => {
    expect(formatTime(-2)).toBe('−00:02.000')
    expect(formatTime(-0.0001)).toBe('00:00.000')
  })
  it('follows the tempo', () => {
    expect(secondsOf(3840, 120)).toBeCloseTo(2)
    expect(secondsOf(3840, 60)).toBeCloseTo(4)
  })
})

describe('tempo text', () => {
  it('accepts the range and a decimal comma', () => {
    expect(parseTempo('120')).toBe(120)
    expect(parseTempo(' 120,5 ')).toBe(120.5)
    expect(parseTempo('20')).toBe(20)
    expect(parseTempo('999')).toBe(999)
  })
  it('refuses what the command would refuse', () => {
    for (const text of ['', ' ', 'abc', '19.9', '999.01', '1e3', '-5', '1,2,3', 'NaN', 'Infinity', '.', '0x20']) {
      expect(parseTempo(text), text).toBeNull()
    }
  })
  it('shows at most two decimals', () => {
    expect(formatTempo(120)).toBe('120')
    expect(formatTempo(120.456)).toBe('120.46')
    expect(formatTempo(99.5)).toBe('99.5')
  })
})

describe('stepTempo', () => {
  it('moves to the next whole BPM and stays inside the range', () => {
    expect(stepTempo(120, 1)).toBe(121)
    expect(stepTempo(120.5, 1)).toBe(121)
    expect(stepTempo(120.5, -1)).toBe(120)
    expect(stepTempo(998.5, 1)).toBe(999)
    expect(stepTempo(999, 1)).toBe(999)
    expect(stepTempo(20.5, -1)).toBe(20)
    expect(stepTempo(20, -1)).toBe(20)
  })
})

describe('parseNumerator', () => {
  it('takes whole numbers from 1 to 32', () => {
    expect(parseNumerator('7')).toBe(7)
    expect(parseNumerator('32')).toBe(32)
    for (const text of ['0', '33', '', '1.5', '-1', 'a']) expect(parseNumerator(text), text).toBeNull()
  })
})

describe('parseBarBeat', () => {
  const four = { numerator: 4, denominator: 4 }
  it('reads a bar, or a bar and a beat, from 1', () => {
    expect(parseBarBeat('1', four)).toBe(0)
    expect(parseBarBeat(' 3 ', four)).toBe(2 * 3840)
    expect(parseBarBeat('3.2', four)).toBe(2 * 3840 + 960)
    expect(parseBarBeat('3,4', four)).toBe(2 * 3840 + 3 * 960)
    expect(parseBarBeat('2.1', { numerator: 3, denominator: 8 })).toBe(3 * 480)
    expect(parseBarBeat('99999', four)).toBe(99998 * 3840)
    expect(parseBarBeat('1.04', four)).toBe(3 * 960)
    expect(parseBarBeat('1.3', { numerator: 3, denominator: 8 })).toBe(2 * 480)
  })
  it('refuses what is no position', () => {
    for (const text of ['', 'a', '0', '1.0', '1.5', '2.', '.5', '-1', '1.2.3', '100000', '1e3']) expect(parseBarBeat(text, four), text).toBeNull()
    expect(parseBarBeat('1.4', { numerator: 3, denominator: 8 })).toBeNull()
  })
})
