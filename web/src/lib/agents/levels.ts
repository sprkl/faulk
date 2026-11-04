// What an agent without ears reads of the sound (#665): the live meters held between two calls of the mixer levels query,
// and the offline measure of `project.analyze_render`. The numbers are the page's; the commands
// (`crates/commands/src/levels.rs`) turn them into dBFS and name the tracks.
import type { FeedbackReport, Meter } from '../engine/feedback'
import type { ContentLevels, FromExport, ToExport } from '../export/export-render'
import type { ExportWorkerPort } from '../export/audio-export.svelte'
import type { LevelsStage, MeterReading } from '../project/core'

/** The longest a reading is held, in ms: a call covers at most this much of what was heard. */
export const HOLD_MS = 10_000
/** A report older than this is not the engine's now (the meters of the page fall silent after the same time). */
export const STALE_MS = 500
/** The most an offline measure may take, in ms (the helper gives an agent's call 60 s); past it the Worker is stopped and the agent told. */
export const ANALYSIS_CAP_MS = 50_000

const SILENT: MeterReading = { peak_l: 0, peak_r: 0, rms_l: 0, rms_r: 0 }

const reading = (m: Meter): MeterReading => ({ peak_l: m.peakL, peak_r: m.peakR, rms_l: m.rmsL, rms_r: m.rmsR })

/** The larger of each figure: the loudest peak and the loudest RMS. */
function louder(a: MeterReading, b: MeterReading): MeterReading {
  return {
    peak_l: Math.max(a.peak_l, b.peak_l),
    peak_r: Math.max(a.peak_r, b.peak_r),
    rms_l: Math.max(a.rms_l, b.rms_l),
    rms_r: Math.max(a.rms_r, b.rms_r),
  }
}

interface Bucket {
  second: number
  epoch: number
  tracks: MeterReading[]
  master: MeterReading
}

/**
 * Holds what the engine's reports (30 per second) showed, by slot: the largest peak and the loudest RMS of each
 * strip, in buckets of one second, so that a call covers the last `HOLD_MS` and a rest between two notes does not
 * read as silence. `take` empties it: the next call covers what happened after this one.
 */
export class LevelHold {
  private buckets: Bucket[] = []
  private lastAt = Number.NEGATIVE_INFINITY
  private playing = false

  /** Folds one report, received at `now` (ms). */
  feed(report: FeedbackReport, now: number) {
    const second = Math.floor(now / 1000)
    let bucket = this.buckets.at(-1)
    if (!bucket || bucket.second !== second || bucket.epoch !== report.epoch) {
      bucket = { second, epoch: report.epoch, tracks: [], master: SILENT }
      this.buckets.push(bucket)
      const oldest = now - HOLD_MS
      this.buckets = this.buckets.filter((b) => (b.second + 1) * 1000 > oldest)
    }
    report.tracks.forEach((m, slot) => {
      bucket.tracks[slot] = louder(bucket.tracks[slot] ?? SILENT, reading(m))
    })
    bucket.master = louder(bucket.master, reading(report.master))
    this.lastAt = now
    this.playing = report.transport !== 'stopped'
  }

  /**
   * What was held since the last call, for the slot table of `epoch`; silent when no report came in the last
   * `STALE_MS` (the engine stopped or the link went), and the reports of another epoch (another slot table) left out.
   */
  take(now: number, epoch: number | undefined): LevelsStage {
    const buckets = this.buckets.filter((b) => b.epoch === epoch)
    this.buckets = []
    if (epoch === undefined || now - this.lastAt > STALE_MS) return { playing: false, tracks: [], master: SILENT }
    // Nothing was heard while the transport did not run (the command says so too).
    if (!this.playing) return { playing: false, tracks: [], master: SILENT }
    const stage: LevelsStage = { playing: true, tracks: [], master: SILENT }
    for (const bucket of buckets) {
      bucket.tracks.forEach((m, slot) => {
        stage.tracks[slot] = louder(stage.tracks[slot] ?? SILENT, m)
      })
      stage.master = louder(stage.master, bucket.master)
    }
    // A slot nothing reported on is a hole in a sparse array: silent.
    for (let slot = 0; slot < stage.tracks.length; slot++) stage.tracks[slot] ??= SILENT
    return stage
  }
}

/** How an offline measure ended. */
export type AnalysisOutcome = { status: 'done'; levels: ContentLevels } | { status: 'busy' } | { status: 'aborted' } | { status: 'failed'; reason: string }

/** What a measure needs of the browser: the Worker and the engine module (the export's, `export/browser.ts`). */
export interface AnalyzerDeps {
  spawn(): ExportWorkerPort
  engineModule(): Promise<ArrayBuffer>
}

/** What the measure needs of a prepared render (`ExportJob` when ready). */
export interface AnalysisJob {
  plan: { body_frames: number; settings: { sample_rate: number } }
  setup: Uint8Array
  start: Uint8Array
  stop: Uint8Array
}

/**
 * Runs the offline measure of `project.analyze_render` in the export's Worker (`analysis: true`): one at a time,
 * stopped by the agent's signal or by `capMs`. Stopping terminates the Worker, so nothing is left running or kept;
 * the project is never touched.
 */
export class RenderAnalyzer {
  private running = false

  constructor(
    private readonly deps: AnalyzerDeps,
    private readonly capMs = ANALYSIS_CAP_MS,
  ) {}

  get busy(): boolean {
    return this.running
  }

  async run(job: AnalysisJob, folder: FileSystemDirectoryHandle | null, signal?: AbortSignal): Promise<AnalysisOutcome> {
    if (this.running) return { status: 'busy' }
    this.running = true
    try {
      const module = await this.deps.engineModule()
      if (signal?.aborted) return { status: 'aborted' }
      const worker = this.deps.spawn()
      return await new Promise<AnalysisOutcome>((resolve) => {
        const onAbort = () => finish({ status: 'aborted' })
        const finish = (outcome: AnalysisOutcome) => {
          clearTimeout(timer)
          signal?.removeEventListener('abort', onAbort)
          worker.onmessage = null
          worker.onerror = null
          worker.terminate()
          resolve(outcome)
        }
        const timer = setTimeout(() => finish({ status: 'failed', reason: `the render took longer than ${this.capMs / 1000} s` }), this.capMs)
        signal?.addEventListener('abort', onAbort, { once: true })
        worker.onmessage = (event: MessageEvent<FromExport>) => {
          const message = event.data
          if (message.type === 'done') {
            finish(message.levels ? { status: 'done', levels: message.levels } : { status: 'failed', reason: 'the engine measured nothing' })
          } else if (message.type === 'error') {
            finish({ status: 'failed', reason: `the render failed: ${message.message}` })
          }
        }
        worker.onerror = (event) => finish({ status: 'failed', reason: `the render failed: ${event.message}` })
        const setup = job.setup.slice().buffer
        const start0 = job.start.slice().buffer
        const stop = job.stop.slice().buffer
        const start: ToExport = {
          type: 'start',
          module,
          sampleRate: job.plan.settings.sample_rate,
          bits: 24,
          bodyFrames: job.plan.body_frames,
          setup,
          start: start0,
          stop,
          folder,
          analysis: true,
        }
        worker.postMessage(start, [module, setup, start0, stop])
      })
    } catch (error) {
      return { status: 'failed', reason: `the render could not start: ${String(error)}` }
    } finally {
      this.running = false
    }
  }
}
