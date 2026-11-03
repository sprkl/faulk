import { describe, expect, it } from 'vitest'
import { contextFrameAt, withFrame } from './clock'
import { allNotesOff, inputNoteOff, inputNoteOn } from '../synth-commands'

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
