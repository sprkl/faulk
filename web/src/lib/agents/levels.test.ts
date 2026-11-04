// What an agent reads of the sound (#665): the meters held between two calls, and the offline measure's
// handling of its Worker (one at a time, a cap, a stop).
import { describe, expect, it, vi } from 'vitest'
import type { FeedbackReport, Meter } from '../engine/feedback'
import type { ExportWorkerPort } from '../export/audio-export.svelte'
import type { ContentLevels, FromExport, ToExport } from '../export/export-render'
import { HOLD_MS, LevelHold, RenderAnalyzer, STALE_MS, type AnalysisJob } from './levels'

const meter = (peak: number, rms = peak / 2): Meter => ({ peakL: peak, peakR: peak, rmsL: rms, rmsR: rms, gainReductionDb: 0 })

function report(tracks: number[], master: number, extra: Partial<FeedbackReport> = {}): FeedbackReport {
  return { epoch: 1, transport: 'playing', tracks: tracks.map((p) => meter(p)), master: meter(master), ...extra } as FeedbackReport
}

describe('the held levels', () => {
  it('keeps the loudest peak and RMS of every slot since the last call, and empties on a call', () => {
    const hold = new LevelHold()
    hold.feed(report([0.2, 0.9], 0.5), 1000)
    hold.feed(report([0.8, 0.1], 0.4), 1100)
    const stage = hold.take(1200, 1)
    expect(stage.playing).toBe(true)
    expect(stage.tracks.map((t) => t.peak_l)).toEqual([0.8, 0.9])
    expect(stage.tracks[0].rms_l).toBe(0.4)
    expect(stage.master.peak_l).toBe(0.5)
    // Emptied: nothing was heard since.
    expect(hold.take(1250, 1).tracks).toEqual([])
  })

  it('is silent when the engine stopped reporting, or for another slot table', () => {
    const hold = new LevelHold()
    hold.feed(report([0.9], 0.9), 1000)
    expect(hold.take(1000 + STALE_MS + 1, 1)).toEqual({ playing: false, tracks: [], master: { peak_l: 0, peak_r: 0, rms_l: 0, rms_r: 0 } })
    hold.feed(report([0.9], 0.9), 2000)
    expect(hold.take(2100, 2).tracks).toEqual([])
    hold.feed(report([0.9], 0.9), 3000)
    expect(hold.take(3100, undefined).tracks).toEqual([])
  })

  it('covers the last seconds only, and says when the transport was stopped', () => {
    const hold = new LevelHold()
    hold.feed(report([0.9], 0.9), 1000)
    hold.feed(report([0.1], 0.1, { transport: 'stopped' }), 1000 + HOLD_MS + 2000)
    const stage = hold.take(1000 + HOLD_MS + 2100, 1)
    expect(stage.playing).toBe(false)
    expect(stage.tracks).toEqual([])
    hold.feed(report([0.1], 0.1), 1000 + HOLD_MS + 2200)
    expect(hold.take(1000 + HOLD_MS + 2300, 1).tracks[0].peak_l).toBe(0.1)
  })
})

const job: AnalysisJob = {
  plan: { body_frames: 48_000, settings: { sample_rate: 48_000 } },
  setup: new Uint8Array([1]),
  start: new Uint8Array([3]),
  stop: new Uint8Array([2]),
}
const levels: ContentLevels = { frames: 48_000, tracks: [], master: { peak_l: 1, peak_r: 1, rms_l: 0.5, rms_r: 0.5 }, silenced: 0, reduction: 0 }

class FakeWorker implements ExportWorkerPort {
  onmessage: ((event: MessageEvent<FromExport>) => void) | null = null
  onerror: ((event: ErrorEvent) => void) | null = null
  terminated = false
  started: ToExport | null = null
  postMessage(message: ToExport) {
    this.started = message
  }
  terminate() {
    this.terminated = true
  }
  say(message: FromExport) {
    this.onmessage?.({ data: message } as MessageEvent<FromExport>)
  }
}

function analyzer(capMs?: number) {
  const workers: FakeWorker[] = []
  const deps = {
    spawn: () => {
      const w = new FakeWorker()
      workers.push(w)
      return w
    },
    engineModule: async () => new ArrayBuffer(8),
  }
  return { analyzer: new RenderAnalyzer(deps, capMs), workers }
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('the offline measure', () => {
  it('asks the Worker for a measure only, and answers with the levels it posts', async () => {
    const { analyzer: a, workers } = analyzer()
    const run = a.run(job, null)
    await tick()
    expect(a.busy).toBe(true)
    expect(workers[0].started).toMatchObject({ type: 'start', analysis: true, bodyFrames: 48_000 })
    workers[0].say({
      type: 'done',
      header: new ArrayBuffer(0),
      written: 0,
      peak: 1,
      clipped: 0,
      reduction: 0,
      tailCut: false,
      silenced: 0,
      missing: [],
      levels,
    })
    expect(await run).toEqual({ status: 'done', levels })
    expect(workers[0].terminated).toBe(true)
    expect(a.busy).toBe(false)
  })

  it('refuses a second measure while one runs, and runs again after', async () => {
    const { analyzer: a, workers } = analyzer()
    const first = a.run(job, null)
    await tick()
    expect(await a.run(job, null)).toEqual({ status: 'busy' })
    workers[0].say({ type: 'error', message: 'boom', code: null })
    expect(await first).toMatchObject({ status: 'failed' })
    const again = a.run(job, null)
    await tick()
    workers[1].say({
      type: 'done',
      header: new ArrayBuffer(0),
      written: 0,
      peak: 0,
      clipped: 0,
      reduction: 0,
      tailCut: false,
      silenced: 0,
      missing: [],
      levels,
    })
    expect((await again).status).toBe('done')
  })

  it('stops the Worker when the call is cancelled', async () => {
    const { analyzer: a, workers } = analyzer()
    const controller = new AbortController()
    const run = a.run(job, null, controller.signal)
    await tick()
    controller.abort()
    expect(await run).toEqual({ status: 'aborted' })
    expect(workers[0].terminated).toBe(true)
    expect(a.busy).toBe(false)
    // Already cancelled: no Worker at all.
    expect(await a.run(job, null, controller.signal)).toEqual({ status: 'aborted' })
    expect(workers).toHaveLength(1)
  })

  it('stops the Worker at the time cap', async () => {
    vi.useFakeTimers()
    try {
      const { analyzer: a, workers } = analyzer(5000)
      const run = a.run(job, null)
      await vi.advanceTimersByTimeAsync(10)
      await vi.advanceTimersByTimeAsync(5000)
      expect(await run).toMatchObject({ status: 'failed', reason: expect.stringContaining('5 s') })
      expect(workers[0].terminated).toBe(true)
    } finally {
      vi.useRealTimers()
    }
  })

  it('fails, rather than answer, when the engine measured nothing', async () => {
    const { analyzer: a, workers } = analyzer()
    const run = a.run(job, null)
    await tick()
    workers[0].say({
      type: 'done',
      header: new ArrayBuffer(0),
      written: 0,
      peak: 0,
      clipped: 0,
      reduction: 0,
      tailCut: false,
      silenced: 0,
      missing: [],
      levels: null,
    })
    expect(await run).toMatchObject({ status: 'failed' })
  })
})
