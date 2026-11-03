import { describe, expect, it } from 'vitest'
import { NO_TAPS, TAP_MAX_INTERVALS, TAP_RESET_MS, tap, type TapState } from './tap-tempo'

/** Taps at these times, from a fresh series. */
const tapAt = (...times: number[]): TapState => times.reduce(tap, NO_TAPS)

describe('tap', () => {
  it('gives no tempo for a single tap', () => {
    expect(tapAt(1000)).toEqual({ taps: [1000], bpm: null })
  })

  it('applies from the second tap: 500 ms is 120 BPM', () => {
    expect(tapAt(0, 500).bpm).toBe(120)
  })

  it('averages regular taps', () => {
    expect(tapAt(0, 600, 1200, 1800, 2400).bpm).toBe(100)
  })

  it('averages irregular taps over the whole series, not the last interval', () => {
    // Intervals 500, 700, 600: mean 600 ms, 100 BPM.
    expect(tapAt(0, 500, 1200, 1800).bpm).toBe(100)
  })

  it('rounds to 0.1 BPM', () => {
    // 60000 / 487 = 123.18... BPM
    expect(tapAt(0, 487).bpm).toBe(123.2)
    // 60000 / 513 = 116.95... BPM
    expect(tapAt(0, 513).bpm).toBe(117)
  })

  it('looks at the last 8 intervals only', () => {
    // Four slow taps (1000 ms), then nine at 500 ms: the slow ones are out of the window.
    const times = [0, 1000, 2000, 3000]
    for (let i = 1; i <= TAP_MAX_INTERVALS + 1; i++) times.push(3000 + i * 500)
    const state = tapAt(...times)
    expect(state.taps).toHaveLength(TAP_MAX_INTERVALS + 1)
    expect(state.bpm).toBe(120)
  })

  it('starts over after a pause longer than 2 s, and the new first tap gives no tempo', () => {
    const before = tapAt(0, 500, 1000)
    expect(before.bpm).toBe(120)
    const restart = 1000 + TAP_RESET_MS + 1
    const after = tap(before, restart)
    expect(after).toEqual({ taps: [restart], bpm: null })
    expect(tap(after, restart + 400).bpm).toBe(150)
  })

  it('keeps the series at exactly 2 s', () => {
    expect(tapAt(0, TAP_RESET_MS).bpm).toBe(30)
  })

  it('clamps to the project limits', () => {
    // 1 ms between taps is 60000 BPM: the top of the range, 999.
    expect(tapAt(0, 1).bpm).toBe(999)
    expect(tapAt(0, 10).bpm).toBe(999)
  })

  it('ignores a second tap in the same millisecond and restarts if the clock goes back', () => {
    const state = tapAt(100, 600)
    expect(tap(state, 600)).toBe(state)
    expect(tap(state, 50)).toEqual({ taps: [50], bpm: null })
  })

  it('does not change the state it was given', () => {
    const state = tapAt(0, 500)
    const copy = JSON.stringify(state)
    tap(state, 1000)
    expect(JSON.stringify(state)).toBe(copy)
  })
})
