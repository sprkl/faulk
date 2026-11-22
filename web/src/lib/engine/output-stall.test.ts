import { describe, expect, it } from 'vitest'
import { OutputStall, STALL_MS, STALL_TICKS } from './output-stall'

/** A page timer that reads the clock every `step` ms; `run` says whether the audio clock moves meanwhile. */
function harness(step = 33) {
  const stall = new OutputStall()
  let now = 0
  let clock = 0
  const tick = (run: boolean, expected = true, dt = step) => {
    now += dt
    if (run) clock += dt / 1000
    return stall.observe({ now, clock, expected })
  }
  /** Ticks for `ms` and returns the readings. */
  const during = (ms: number, run: boolean, expected = true, dt = step) => {
    const out: boolean[] = []
    for (let t = 0; t < ms; t += dt) out.push(tick(run, expected, dt))
    return out
  }
  return { stall, tick, during }
}

describe('OutputStall (#810)', () => {
  it('never reports a stall while the clock advances', () => {
    const { during } = harness()
    expect(during(10_000, true).some(Boolean)).toBe(false)
  })

  it('reports a stall once the clock stood still for STALL_MS, not before', () => {
    const { tick, during } = harness()
    during(500, true)
    const still = during(STALL_MS, false)
    expect(still.every((s) => !s)).toBe(true)
    // The first unchanged reading starts the count, so one more reading crosses STALL_MS.
    expect(tick(false)).toBe(true)
    expect(tick(false)).toBe(true)
  })

  it('clears at the first reading where the clock moved again (a context resumed later)', () => {
    const { tick, during } = harness()
    during(2000, false)
    expect(tick(false)).toBe(true)
    expect(tick(true)).toBe(false)
    // The next stall counts from scratch.
    expect(during(STALL_MS - 100, false).some(Boolean)).toBe(false)
  })

  it('a single late reading with the clock unchanged is no stall (sleep, long task, throttled timer)', () => {
    const { tick } = harness()
    tick(true)
    // The computer slept an hour: the page clock moved on, the audio clock did not run.
    expect(tick(false, true, 3_600_000)).toBe(false)
    // The output takes 300 ms to come back: still no stall, the count starts after the gap.
    for (let t = 0; t < 300; t += 33) expect(tick(false)).toBe(false)
    expect(tick(true)).toBe(false)
  })

  it('a background tab, read once a second while the clock runs, is no stall', () => {
    const { during } = harness()
    expect(during(60_000, true, true, 1000).some(Boolean)).toBe(false)
  })

  it('a background tab whose output stalled reports it after STALL_TICKS readings', () => {
    const { tick } = harness(1000)
    tick(true)
    const seen: boolean[] = []
    for (let i = 0; i < STALL_TICKS + 1; i++) seen.push(tick(false))
    // The first unchanged reading starts the count: the STALL_TICKS-th one is a second after it, and says it.
    expect(seen.slice(0, STALL_TICKS - 1).some(Boolean)).toBe(false)
    expect(seen[STALL_TICKS - 1]).toBe(true)
    expect(seen[STALL_TICKS]).toBe(true)
  })

  it('needs STALL_TICKS readings even when they span more than STALL_MS', () => {
    const { tick } = harness()
    tick(true)
    expect(tick(false, true, 5000)).toBe(false)
    expect(tick(false, true, 5000)).toBe(false)
    expect(tick(false, true, 5000)).toBe(true)
  })

  it('a context the page suspended (Boost plays), or not started yet, is never stalled, and resuming counts again', () => {
    const { tick, during } = harness()
    // Before the context started: not expected.
    expect(during(3000, false, false).some(Boolean)).toBe(false)
    during(500, true)
    expect(during(5000, false, false).some(Boolean)).toBe(false)
    // Resumed: the readings of the suspension are forgotten, a new stall needs its own STALL_MS.
    expect(tick(false)).toBe(false)
    expect(during(STALL_MS - 100, false).some(Boolean)).toBe(false)
    expect(during(200, false).some(Boolean)).toBe(true)
  })

  it('a stall cleared by the page suspending the context reads as no stall', () => {
    const { tick, during } = harness()
    during(2000, false)
    expect(tick(false)).toBe(true)
    expect(tick(false, false)).toBe(false)
  })

  it('a page clock that went back, or a non-finite reading, starts again', () => {
    const stall = new OutputStall()
    for (let i = 0; i < 100; i++) stall.observe({ now: i * 33, clock: 1, expected: true })
    expect(stall.stalled).toBe(true)
    expect(stall.observe({ now: 0, clock: 1, expected: true })).toBe(false)
    expect(stall.observe({ now: 50, clock: Number.NaN, expected: true })).toBe(false)
  })
})
