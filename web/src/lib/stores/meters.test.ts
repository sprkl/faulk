import { describe, expect, it } from 'vitest'
import type { FeedbackReport, Meter } from '../engine/feedback'
import type { EngineStore } from './engine.svelte'
import { MASTER_METER, MeterStore } from './meters.svelte'
import type { ProjectStore } from './project.svelte'

const meter = (peak: number, gainReductionDb = 0): Meter => ({ peakL: peak, peakR: peak, rmsL: 0, rmsR: 0, gainReductionDb })

function report(tracks: Meter[], master: Meter, epoch = 1, readouts: FeedbackReport['readouts'] = []): FeedbackReport {
  return { epoch, tracks, master, readouts } as unknown as FeedbackReport
}

function store(ids: string[]) {
  const project = {
    tracks: ids.map((id) => ({ id })),
    view: {
      slots: { epoch: 1, tracks: Object.fromEntries(ids.map((id, i) => [id, i])), devices: { fa: { track: 0, slot: 1 }, fm: { track: null, slot: 0 } } },
    },
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

  it('maps the devices’ readouts to their ids by their engine slots, in the epoch of the slot table (#686)', () => {
    const meters = store(['a'])
    const live = [
      { track: 0, device: 1, param: 1, value: 640 },
      { track: null, device: 0, param: 1, value: 3000 },
      { track: 5, device: 0, param: 1, value: 9 },
    ]
    meters.feed(report([meter(0.1)], meter(0.1), 1, live), 0)
    expect(meters.readout('fa', 1)).toBe(640)
    expect(meters.readout('fm', 1)).toBe(3000)
    expect(meters.readout('fa', 2)).toBeUndefined()
    expect(meters.readouts.size).toBe(2)
    // Another epoch names other slots: nothing; silence: nothing.
    meters.feed(report([meter(0.1)], meter(0.1), 2, live), 33)
    expect(meters.readout('fa', 1)).toBeUndefined()
    meters.feed(report([meter(0.1)], meter(0.1), 1, live), 66)
    meters.feed(null, 1000)
    expect(meters.readout('fa', 1)).toBeUndefined()
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
    const ids = ['a', 'b']
    const project = {
      tracks: ids.map((id) => ({ id })),
      view: { slots: { epoch: 1, tracks: { a: 0, b: 1 } } },
    } as unknown as ProjectStore
    const meters = new MeterStore(project, {} as EngineStore)
    meters.feed(report([meter(1.5), meter(0.2)], meter(0.1)), 0)
    expect(meters.snaps.has('a')).toBe(true)
    expect(meters.snap('a').clipped).toBe(true)
    // The track is deleted: the next feed drops its strip, clip and all.
    ;(project as unknown as { tracks: { id: string }[] }).tracks = [{ id: 'b' }]
    meters.feed(report([meter(0.2)], meter(0.1)), 33)
    expect(meters.snaps.has('a')).toBe(false)
    expect(meters.snap('a').clipped).toBe(false)
    expect(meters.snaps.has('b')).toBe(true)
  })
})
