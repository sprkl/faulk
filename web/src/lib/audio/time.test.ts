import { describe, expect, it } from 'vitest'
import { framesPerTick, framesToTicks } from './time'

describe('audio frames on the timeline (AU-6)', () => {
  it('gives the ticks the project gives: frames × bpm × 960 / (60 × rate), to the nearest tick, at least one', () => {
    // The values `crates/commands` computes (tests/it/arrangement_audio.rs and arrangement.rs).
    expect(framesToTicks(44_100, 44_100, 120)).toBe(1920)
    expect(framesToTicks(22_050, 44_100, 120)).toBe(960)
    expect(framesToTicks(11_025, 44_100, 120)).toBe(480)
    expect(framesToTicks(27_562, 44_100, 120)).toBe(1200)
    expect(framesToTicks(48_000, 48_000, 97.3)).toBe(1557)
    expect(framesToTicks(44_100, 44_100, 60)).toBe(960)
    expect(framesToTicks(44_100, 44_100, 240)).toBe(3840)
    // A frame holds less than a tick: still one tick.
    expect(framesToTicks(1, 96_000, 20)).toBe(1)
    expect(framesToTicks(0, 48_000, 120)).toBe(1)
  })

  it('says how many frames a tick holds', () => {
    expect(framesPerTick(44_100, 120)).toBeCloseTo(22.96875, 9)
    expect(framesPerTick(48_000, 120)).toBe(25)
  })
})
