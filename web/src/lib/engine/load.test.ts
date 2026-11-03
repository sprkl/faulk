import { describe, expect, it } from 'vitest'
import { blockSeconds, MAX_TIMED_BLOCK_MS, renderClock } from './engine-abi'
import { LOAD_SMOOTHING_MS, LoadMeter, PEAK_HOLD_MS } from './load'

describe('LoadMeter (#486)', () => {
  it('starts from the first report, then smooths with the real time between reports', () => {
    const meter = new LoadMeter()
    expect(meter.update(0.2, 0.3, 0)).toEqual({ smoothed: 0.2, peak: 0.3 })
    // One time constant later the average has gone 63 % of the way.
    const after = meter.update(0.8, 0.8, LOAD_SMOOTHING_MS)!
    expect(after.smoothed).toBeCloseTo(0.2 + 0.6 * (1 - Math.exp(-1)))
    // A report at the same instant does not move it; one long after replaces it.
    expect(meter.update(0, 0, LOAD_SMOOTHING_MS)!.smoothed).toBeCloseTo(after.smoothed)
    expect(meter.update(0.1, 0.1, LOAD_SMOOTHING_MS * 20)!.smoothed).toBeCloseTo(0.1)
  })

  it('holds the largest peak for three seconds, never under the smoothed load', () => {
    const meter = new LoadMeter()
    meter.update(0.1, 0.9, 0)
    expect(meter.update(0.1, 0.2, PEAK_HOLD_MS)!.peak).toBe(0.9)
    expect(meter.update(0.1, 0.2, PEAK_HOLD_MS + 1)!.peak).toBe(0.2)
    // A report without a peak counts its load; the peak shown is at least the smoothed load.
    const fresh = new LoadMeter()
    expect(fresh.update(0.5, null, 0)).toEqual({ smoothed: 0.5, peak: 0.5 })
    expect(fresh.update(0.5, 0.1, 10)!.peak).toBe(0.5)
  })

  it('shows nothing while the engine does not measure, and starts again after', () => {
    const meter = new LoadMeter()
    meter.update(0.9, 0.9, 0)
    expect(meter.update(null, null, 10)).toBeNull()
    expect(meter.update(0.1, 0.1, 20)).toEqual({ smoothed: 0.1, peak: 0.1 })
  })

  it('forgets everything on reset', () => {
    const meter = new LoadMeter()
    meter.update(0.9, 1, 0)
    meter.reset()
    expect(meter.update(0.2, 0.2, 5)).toEqual({ smoothed: 0.2, peak: 0.2 })
  })
})

describe('the worklet clock (#486)', () => {
  it('takes performance.now() where it exists and measures its step', () => {
    let t = 0
    const clock = renderClock({ performance: { now: () => (t += 0.005) } })
    expect(clock.step).toBeCloseTo(0.000005)
    expect(clock.now()).toBeGreaterThan(0)
  })

  it('falls back on Date.now() (an AudioWorkletGlobalScope in Chromium) and its 1 ms step', () => {
    const clock = renderClock({})
    expect(clock.step).toBeGreaterThanOrEqual(0.001)
    expect(clock.step).toBeLessThan(0.05)
    expect(Math.abs(clock.now() - Date.now())).toBeLessThan(1000)
  })

  it('takes a clock that does not move in its spins as stepping by 1 ms', () => {
    expect(renderClock({ performance: { now: () => 7 } }, 1000).step).toBe(0.001)
  })

  it('counts a reading that went back or jumped as 0 s', () => {
    expect(blockSeconds(10, 12.5)).toBeCloseTo(0.0025)
    expect(blockSeconds(10, 9)).toBe(0)
    expect(blockSeconds(0, MAX_TIMED_BLOCK_MS)).toBe(0)
    expect(blockSeconds(0, NaN)).toBe(0)
  })
})
