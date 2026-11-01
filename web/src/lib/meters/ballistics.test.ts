import { describe, expect, it } from 'vitest'
import { Ballistics, HOLD_S, METER_FLOOR_DB, MeterBank, RELEASE_DB_PER_S, toDb } from './ballistics'

describe('Ballistics', () => {
  it('attacks at once and releases at a steady rate', () => {
    const b = new Ballistics()
    b.update(0.5, 0)
    expect(b.level).toBeCloseTo(toDb(0.5), 9)
    b.update(0, 0.1)
    expect(b.level).toBeCloseTo(toDb(0.5) - RELEASE_DB_PER_S * 0.1, 9)
    b.update(0, 10)
    // A gap longer than a quarter second counts as a quarter second (a hidden tab).
    expect(b.level).toBeCloseTo(toDb(0.5) - RELEASE_DB_PER_S * 0.35, 9)
    for (let t = 10; t < 20; t += 0.033) b.update(0, t)
    expect(b.level).toBe(METER_FLOOR_DB)
  })

  it('holds a peak for the hold time, then lets it fall', () => {
    const b = new Ballistics()
    b.update(1, 0)
    const top = b.hold
    let t = 0
    while (t < HOLD_S - 0.1) b.update(0, (t += 0.033))
    expect(b.hold).toBe(top)
    while (t < HOLD_S + 1) b.update(0, (t += 0.033))
    expect(b.hold).toBeLessThan(top)
    for (let i = 0; i < 400; i++) b.update(0, (t += 0.033))
    expect(b.hold).toBe(METER_FLOOR_DB)
  })

  it('keeps the highest peak and the clip mark until reset', () => {
    const b = new Ballistics()
    b.update(0.2, 0)
    b.update(1.2, 0.03)
    b.update(0, 5)
    expect(b.peak).toBeCloseTo(toDb(1.2), 9)
    expect(b.clipped).toBe(true)
    b.reset()
    expect(b.peak).toBe(METER_FLOOR_DB)
    expect(b.clipped).toBe(false)
  })

  it('floors silence, negative and non finite peaks', () => {
    for (const bad of [0, -1, NaN, -Infinity]) expect(toDb(bad)).toBe(METER_FLOOR_DB)
    const b = new Ballistics()
    b.update(NaN, 0)
    b.update(Infinity, 0.03)
    expect(Number.isFinite(b.level)).toBe(true)
  })
})

describe('MeterBank', () => {
  const read = () => ({ peakL: 0.5, peakR: 0.25 })

  it('holds one meter per strip and drops the strips a report no longer lists', () => {
    const bank = new MeterBank()
    bank.update(['a', 'b', 'master'], read, 0)
    expect(bank.size).toBe(3)
    const snaps = bank.update(['a', 'master'], read, 0.03)
    expect(bank.size).toBe(2)
    expect([...snaps.keys()]).toEqual(['a', 'master'])
  })

  it('does not grow over a long run of reports', () => {
    const bank = new MeterBank()
    for (let i = 0; i < 20000; i++) bank.update(['a', 'b', 'master'], read, i * 0.033)
    expect(bank.size).toBe(3)
  })

  it('draws the two channels apart, and resets one strip only', () => {
    const bank = new MeterBank()
    const snaps = bank.update(['a', 'b'], (id) => (id === 'a' ? { peakL: 1.5, peakR: 0 } : { peakL: 0.1, peakR: 0.1 }), 0)
    expect(snaps.get('a')!.levelL).toBeGreaterThan(snaps.get('a')!.levelR)
    expect(snaps.get('a')!.clipped).toBe(true)
    expect(bank.reset('a').clipped).toBe(false)
    expect(bank.reset('b').peakDb).toBe(METER_FLOOR_DB)
    expect(bank.reset('unknown').clipped).toBe(false)
  })
})
