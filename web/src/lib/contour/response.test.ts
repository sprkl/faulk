import { describe, expect, it } from 'vitest'
import fixture from './response.fixture.json'
import { BANDS, bandDb, curveHz, hzOfX, isCut, responseDb, xOfHz } from './response'

describe('the Contour curve', () => {
  it('is the engine response of the fixture, within 0.01 dB', () => {
    let worst = 0
    for (const c of fixture.cases) {
      const params = c.params as Record<string, number>
      const values = (key: string) => params[key]
      for (const [hz, db] of c.points) {
        const mine = responseDb(values, hz, c.sample_rate)
        worst = Math.max(worst, Math.abs(mine - db))
        expect(Math.abs(mine - db), `${hz} Hz at ${c.sample_rate}`).toBeLessThan(0.01)
      }
    }
    expect(worst).toBeLessThan(0.01)
  })

  it('puts a bell at its gain at its frequency, and the cut bands at -3 dB at theirs', () => {
    const values = (gain: number) => (key: string) =>
      ({ bell2_freq: 1000, bell2_gain: gain, bell2_q: 2, hp_freq: 200, hp_q: Math.SQRT1_2, hp_slope: 1 })[key] ?? 0
    const bell = BANDS[3]
    for (const g of [-24, -6, 0, 6, 24]) expect(bandDb(bell, values(g), 1000, 48000)).toBeCloseTo(g, 4)
    const hp = BANDS[0]
    expect(isCut(hp.shape)).toBe(true)
    expect(bandDb(hp, values(0), 200, 48000)).toBeCloseTo(-3.01, 1)
  })

  it('maps the log axis both ways', () => {
    expect(xOfHz(20)).toBe(0)
    expect(xOfHz(20000)).toBeCloseTo(1, 10)
    expect(hzOfX(0.5)).toBeCloseTo(Math.sqrt(20 * 20000), 6)
    expect(hzOfX(xOfHz(1234))).toBeCloseTo(1234, 6)
    // Out of the axis: clamped.
    expect(xOfHz(5)).toBe(0)
    expect(hzOfX(2)).toBe(20000)
    const hz = curveHz(5)
    expect(hz[0]).toBe(20)
    expect(hz[4]).toBeCloseTo(20000, 6)
  })
})
