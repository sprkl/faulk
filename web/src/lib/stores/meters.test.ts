import { describe, expect, it } from 'vitest'
import type { FeedbackReport, Meter } from '../engine/feedback'
import type { EngineStore } from './engine.svelte'
import { MASTER_METER, MeterStore } from './meters.svelte'
import type { ProjectStore } from './project.svelte'

const meter = (peak: number, gainReductionDb = 0): Meter => ({ peakL: peak, peakR: peak, rmsL: 0, rmsR: 0, gainReductionDb })

function report(tracks: Meter[], master: Meter, epoch = 1): FeedbackReport {
  return { epoch, tracks, master } as unknown as FeedbackReport
}

function store(ids: string[]) {
  const project = {
    tracks: ids.map((id) => ({ id })),
    view: { slots: { epoch: 1, tracks: Object.fromEntries(ids.map((id, i) => [id, i])) } },
  } as unknown as ProjectStore
  return new MeterStore(project, {} as EngineStore)
}

describe('MeterStore', () => {
  it('latches a clip on its own strip, whatever view is shown, and resets only that strip', () => {
    const meters = store(['a', 'b'])
    meters.feed(report([meter(1.2), meter(0.1)], meter(0.5)), 0)
    expect(meters.snap('a').clipped).toBe(true)
    expect(meters.snap('b').clipped).toBe(false)
    // Silence after the clip: the latch stays.
    meters.feed(report([meter(0), meter(0)], meter(0)), 5000)
    expect(meters.snap('a').clipped).toBe(true)
    meters.feed(report([meter(0), meter(1)], meter(1)), 5100)
    expect(meters.snap('b').clipped).toBe(true)
    expect(meters.snap(MASTER_METER).clipped).toBe(true)
    meters.reset('a')
    expect(meters.snap('a').clipped).toBe(false)
    expect(meters.snap('b').clipped).toBe(true)
    expect(meters.snap(MASTER_METER).clipped).toBe(true)
  })

  it('carries the gain reduction of a strip and of the master', () => {
    const meters = store(['a'])
    meters.feed(report([meter(0.3, 7.5)], meter(0.3, 2)), 0)
    expect(meters.snap('a').reductionDb).toBe(7.5)
    expect(meters.snap(MASTER_METER).reductionDb).toBe(2)
    meters.feed(report([meter(0.3, -3)], meter(0.3, NaN)), 33)
    expect(meters.snap('a').reductionDb).toBe(0)
    expect(meters.snap(MASTER_METER).reductionDb).toBe(0)
  })

  it('falls to silence without a report, and ignores a report of another epoch', () => {
    const meters = store(['a'])
    meters.feed(report([meter(0.5)], meter(0.5)), 0)
    expect(meters.snap('a').levelDb).toBeGreaterThan(-10)
    meters.feed(report([meter(0.5)], meter(0.5), 2), 33)
    expect(meters.snap('a').levelDb).toBeLessThan(0)
    for (let t = 1; t < 400; t++) meters.feed(null, t * 33)
    expect(meters.snap('a').levelDb).toBe(-60)
    expect(meters.snap('nobody').levelDb).toBe(-60)
  })

  it('drops the strip of a deleted track', () => {
    const meters = store(['a', 'b'])
    meters.feed(report([meter(1.5), meter(0.2)], meter(0.1)), 0)
    const fewer = store(['b'])
    fewer.feed(report([meter(0.2)], meter(0.1)), 0)
    expect(fewer.snaps.has('a')).toBe(false)
    expect(meters.snaps.has('a')).toBe(true)
  })
})
