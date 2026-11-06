import { describe, expect, it } from 'vitest'
import { contextFrameAt, OutputClock, withFrame } from './clock'
import { allNotesOff, inputNoteOff, inputNoteOn } from '../synth-commands'

describe('the smoothed page clock ↔ audio frame correspondence (#725)', () => {
  // The context is 2 s ahead of the page clock's seconds; every pair of a callback carries its own jitter.
  const pair = (perf: number, jitterMs = 0) => ({ contextTime: perf / 1000 + 2 + jitterMs / 1000, performanceTime: perf })

  it('is the pair itself with a single pair, and ignores an unusable one', () => {
    const clock = new OutputClock()
    expect(clock.observe(null)).toBeNull()
    expect(clock.observe({ contextTime: 0, performanceTime: 0 })).toBeNull()
    expect(clock.observe({ contextTime: Number.NaN, performanceTime: 5 })).toBeNull()
    expect(clock.observe(pair(10_000, 0.7))).toEqual(pair(10_000, 0.7))
  })

  it('drops the jitter of single pairs: the median offset of the window', () => {
    const clock = new OutputClock()
    for (let k = 0; k < 20; k++) clock.observe(pair(10_000 + k * 5, k % 7 === 3 ? 1.4 : 0))
    const out = clock.observe(pair(10_105, 1.4))!
    expect(out.performanceTime).toBe(10_105)
    expect(out.contextTime).toBeCloseTo(10.105 + 2, 9)
  })

  it('follows a drift as old pairs leave the window, and starts again when the performance clock goes back', () => {
    const clock = new OutputClock()
    let last = { contextTime: 0, performanceTime: 0 }
    // 1 ms of drift a second, for 5 s.
    for (let k = 0; k <= 1000; k++) last = clock.observe(pair(10_000 + k * 5, (k * 5) / 1000))!
    expect(last.contextTime - (last.performanceTime / 1000 + 2)).toBeCloseTo(0.0045, 3)
    expect(clock.observe(pair(5_000, 3))).toEqual(pair(5_000, 3))
  })

  it('starts again at once on a step of the offset (a resume after a suspension), not after the window', () => {
    const clock = new OutputClock()
    for (let k = 0; k < 30; k++) clock.observe(pair(10_000 + k * 5))
    // The context paused for 300 ms: its time is 0.3 s behind for the same performance time.
    const step = { contextTime: 10.15 + 2 - 0.3, performanceTime: 10_150 }
    expect(clock.observe(step)).toEqual(step)
    const next = clock.observe({ contextTime: 10.155 + 2 - 0.3, performanceTime: 10_155 })!
    expect(next.contextTime).toBeCloseTo(10.155 + 2 - 0.3, 9)
    // A jitter of 3.5 ms is not a step.
    const kept = clock.observe({ contextTime: 10.16 + 2 - 0.3 + 0.0035, performanceTime: 10_160 })!
    expect(kept.contextTime).toBeCloseTo(10.16 + 2 - 0.3, 9)
  })

  it('answers the same pair twice with the same offset', () => {
    const clock = new OutputClock()
    clock.observe(pair(10_000))
    const a = clock.observe(pair(10_005, 1))
    expect(clock.observe(pair(10_005, 1))).toEqual(a)
  })
})

describe('the frame a live note was played at (#499)', () => {
  // Heard: context time 2 s at performance time 10 000 ms; 48 kHz; 20 ms of output latency.
  const stamp = { contextTime: 2, performanceTime: 10_000 }

  it('is the frame heard at that instant plus the output latency: the frame being rendered then', () => {
    expect(contextFrameAt(stamp, 10_000, 0, 48_000)).toBe(96_000)
    // 500 ms later: 24 000 frames later, whether before or after the output timestamp.
    expect(contextFrameAt(stamp, 10_500, 0, 48_000)).toBe(120_000)
    expect(contextFrameAt(stamp, 9_500, 0, 48_000)).toBe(72_000)
    // The latency moves it later by as much (960 frames): the take's shift moves it back by the same.
    expect(contextFrameAt(stamp, 10_000, 0.02, 48_000)).toBe(96_960)
    // A fraction of a frame rounds to the nearest.
    expect(contextFrameAt(stamp, 10_000.01, 0, 48_000)).toBe(96_000)
    expect(contextFrameAt(stamp, 10_000.02, 0, 48_000)).toBe(96_001)
  })

  it('is unknown without a time, before the context output anything, or before its first frame', () => {
    expect(contextFrameAt(stamp, undefined, 0, 48_000)).toBeNull()
    expect(contextFrameAt(stamp, Number.NaN, 0, 48_000)).toBeNull()
    expect(contextFrameAt({ contextTime: 0, performanceTime: 0 }, 10_000, 0, 48_000)).toBeNull()
    expect(contextFrameAt(null, 10_000, 0, 48_000)).toBeNull()
    expect(contextFrameAt({}, 10_000, 0, 48_000)).toBeNull()
    expect(contextFrameAt(stamp, 7_000, 0, 48_000)).toBeNull()
    // A latency that is no number is left out rather than spoiling the frame.
    expect(contextFrameAt(stamp, 10_000, Number.NaN, 48_000)).toBe(96_000)
  })

  it('goes into the live input notes only', () => {
    expect(withFrame(inputNoteOn(60, 100), 96_000)).toEqual({ type: 'live.input_note_on', note: 60, velocity: 100, frame: 96_000 })
    expect(withFrame(inputNoteOff(60), 96_000)).toEqual({ type: 'live.input_note_off', note: 60, frame: 96_000 })
    expect(withFrame(inputNoteOn(60, 100), null)).toEqual(inputNoteOn(60, 100))
    expect(withFrame(allNotesOff(), 96_000)).toEqual(allNotesOff())
  })
})
