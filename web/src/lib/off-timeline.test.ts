import { describe, expect, it } from 'vitest'
import type { FeedbackReport } from './engine/feedback'
import { offTimeline } from './off-timeline'

const slots = { epoch: 3, tracks: { a: 0, b: 1, c: 2 }, scenes: { s: 0 }, devices: {} }
const report = (clips: FeedbackReport['clips'], held: number[], epoch = 3) => ({ epoch, clips, held })
const cell = (track: number, state: FeedbackReport['clips'][number]['state']) => ({ track, scene: 0, state, anchor: 0 })

describe('offTimeline', () => {
  it('holds the tracks the grid holds without a clip and the ones that play or stop a clip', () => {
    const off = offTimeline(report([cell(0, 'playing'), cell(1, 'stopping')], [2]), slots)
    expect([...off].sort()).toEqual(['a', 'b', 'c'])
  })

  it('does not count a waiting launch: the track still follows its timeline', () => {
    expect(offTimeline(report([cell(0, 'queued'), cell(1, 'armed')], []), slots).size).toBe(0)
  })

  it('is empty with no report, no slots, another epoch or an unknown slot', () => {
    expect(offTimeline(null, slots).size).toBe(0)
    expect(offTimeline(report([], [0]), undefined).size).toBe(0)
    expect(offTimeline(report([], [0], 2), slots).size).toBe(0)
    expect(offTimeline(report([cell(9, 'playing')], [8]), slots).size).toBe(0)
  })
})
