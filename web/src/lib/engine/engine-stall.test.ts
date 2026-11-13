import { describe, expect, it } from 'vitest'
import { EngineStall, STALL_CLOCK_S, STALL_TICKS } from './engine-stall'

/**
 * A page timer that reads every `step` ms. `clock`: whether the audio clock runs meanwhile; `reports`: whether the engine
 * wrote a new report meanwhile.
 */
function harness(step = 33) {
  const stall = new EngineStall()
  let now = 0
  let clock = 0
  let seq = 0
  const tick = ({ clock: runs = true, reports = true, expected = true, dt = step } = {}) => {
    now += dt
    if (runs) clock += dt / 1000
    if (reports) seq += 2
    return stall.observe({ now, clock, seq, expected })
  }
  /** Ticks for `ms` and returns the readings. */
  const during = (ms: number, options: Parameters<typeof tick>[0] = {}) => {
    const out: boolean[] = []
    const dt = options.dt ?? step
    for (let t = 0; t < ms; t += dt) out.push(tick(options))
    return out
  }
  return { stall, tick, during }
}

const stuck = { reports: false }

describe('EngineStall (#877)', () => {
  it('never reports a stall while the engine reports', () => {
    const { during } = harness()
    expect(during(10_000).some(Boolean)).toBe(false)
  })

  it('reports a stall once the audio clock ran STALL_CLOCK_S without a new report, not before', () => {
    const { tick, during } = harness()
    during(500)
    // The reading where the last report was seen is the baseline: STALL_CLOCK_S of audio clock from there.
    const still = during(STALL_CLOCK_S * 1000 - 33, stuck)
    expect(still.every((s) => !s)).toBe(true)
    expect(tick(stuck)).toBe(true)
    expect(tick(stuck)).toBe(true)
  })

  it('clears at the first reading that finds a new report, and the next stall counts from scratch', () => {
    const { tick, during } = harness()
    during(2000, stuck)
    expect(tick(stuck)).toBe(true)
    expect(tick()).toBe(false)
    expect(during(STALL_CLOCK_S * 1000 - 100, stuck).some(Boolean)).toBe(false)
  })

  it('a clock that stands still is never an engine stall: that one is the output’s (#810)', () => {
    const { tick, during } = harness()
    during(500)
    expect(during(10_000, { clock: false, reports: false }).some(Boolean)).toBe(false)
    // The clock runs again but the engine still does not report: the count starts where the clock moves.
    expect(during(STALL_CLOCK_S * 1000 - 100, stuck).some(Boolean)).toBe(false)
    expect(during(200, stuck).some(Boolean)).toBe(true)
    // The clock stops again while stalled: the word stays (the shell lets the output's word win).
    expect(tick({ clock: false, reports: false })).toBe(true)
  })

  it('a late reading that finds a new report is no stall (sleep, long task, throttled timer)', () => {
    const { tick } = harness()
    tick()
    expect(tick({ dt: 3_600_000 })).toBe(false)
    expect(tick()).toBe(false)
  })

  it('a background tab, read once a second while the engine reports, is no stall', () => {
    const { during } = harness()
    expect(during(60_000, { dt: 1000 }).some(Boolean)).toBe(false)
  })

  it('a background tab whose engine stopped reports it after STALL_TICKS readings', () => {
    const { tick } = harness(1000)
    tick()
    const seen: boolean[] = []
    for (let i = 0; i < STALL_TICKS + 1; i++) seen.push(tick(stuck))
    expect(seen.slice(0, STALL_TICKS - 1).some(Boolean)).toBe(false)
    expect(seen[STALL_TICKS - 1]).toBe(true)
    expect(seen[STALL_TICKS]).toBe(true)
  })

  it('needs STALL_TICKS readings even when the clock jumped far past STALL_CLOCK_S', () => {
    const { tick } = harness()
    tick()
    expect(tick({ reports: false, dt: 5000 })).toBe(false)
    expect(tick({ reports: false, dt: 5000 })).toBe(false)
    expect(tick({ reports: false, dt: 5000 })).toBe(true)
  })

  it('an engine never expected to render (not started, suspended for Boost) is never stalled, and resuming counts again', () => {
    const { tick, during } = harness()
    expect(during(3000, { reports: false, expected: false }).some(Boolean)).toBe(false)
    during(500)
    expect(during(5000, { reports: false, expected: false }).some(Boolean)).toBe(false)
    // Resumed: the readings of the suspension are forgotten, a new stall needs its own STALL_CLOCK_S.
    expect(tick(stuck)).toBe(false)
    expect(during(STALL_CLOCK_S * 1000 - 100, stuck).some(Boolean)).toBe(false)
    expect(during(200, stuck).some(Boolean)).toBe(true)
  })

  it('a stall cleared by the page suspending the context reads as no stall', () => {
    const { tick, during } = harness()
    during(2000, stuck)
    expect(tick(stuck)).toBe(true)
    expect(tick({ reports: false, expected: false })).toBe(false)
  })

  it('an engine that never wrote a report (seq 0 throughout) is said too', () => {
    const stall = new EngineStall()
    const seen = Array.from({ length: 40 }, (_, i) => stall.observe({ now: i * 33, clock: i * 0.033, seq: 0, expected: true }))
    expect(seen.some(Boolean)).toBe(true)
  })

  /** An EngineStall stalled on `seq` 4, from page time 0 and audio clock `from`. */
  function stalledFrom(from = 0) {
    const stall = new EngineStall()
    for (let i = 0; i < 100; i++) stall.observe({ now: i * 33, clock: from + i * 0.033, seq: 4, expected: true })
    expect(stall.stalled).toBe(true)
    return stall
  }
  /** Feeds unchanged readings from (`now`, `clock`) on and returns the first index that says stalled (-1: none). */
  const firstStall = (stall: EngineStall, now: number, clock: number) =>
    Array.from({ length: 60 }, (_, i) => stall.observe({ now: now + i * 33, clock: clock + i * 0.033, seq: 4, expected: true })).indexOf(true)

  it('a page clock that went back starts a new baseline: a new stall needs its own STALL_CLOCK_S', () => {
    const stall = stalledFrom()
    expect(stall.observe({ now: 0, clock: 3.3, seq: 4, expected: true })).toBe(false)
    expect(firstStall(stall, 33, 3.333)).toBe(Math.ceil(STALL_CLOCK_S / 0.033) - 1)
  })

  it('an audio clock that went back (a new context) starts a new baseline', () => {
    const stall = stalledFrom(4)
    expect(stall.observe({ now: 4000, clock: 0.5, seq: 4, expected: true })).toBe(false)
    expect(firstStall(stall, 4033, 0.533)).toBe(Math.ceil(STALL_CLOCK_S / 0.033) - 1)
  })

  it('a non-finite reading forgets everything: the next reading is a baseline, not a still one', () => {
    const stall = stalledFrom()
    expect(stall.observe({ now: 3400, clock: Number.NaN, seq: 4, expected: true })).toBe(false)
    expect(stall.observe({ now: 3433, clock: Number.POSITIVE_INFINITY, seq: 4, expected: true })).toBe(false)
    expect(stall.observe({ now: 3466, clock: 3.4, seq: 4, expected: true })).toBe(false)
    // Same seq as before the NaN, the clock far ahead of the old baseline: still a fresh count.
    expect(firstStall(stall, 3499, 3.433)).toBe(Math.ceil(STALL_CLOCK_S / 0.033) - 1)
  })
})
