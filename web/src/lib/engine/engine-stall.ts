/**
 * Tells when the Web engine stopped responding while the audio output still pulls it (#877): the audio context's clock
 * (`currentTime`) advances, but the engine's report no longer does. The render side writes a report about 30 times per
 * second of audio, whatever the transport does (`Host::render_block`), so a `seq` that stays the same over a second of
 * running audio clock means the render side no longer runs (its `process` threw or trapped and the browser stopped
 * calling it): the position and the meters stay frozen on their last values, and the page says so.
 *
 * It runs on the main thread, from the page's feedback timer, beside `OutputStall` (#810), and keeps its shape: a stall
 * needs `STALL_TICKS` readings in a row that found the same `seq`, and the audio clock advanced by `STALL_CLOCK_S` since
 * the reading where `seq` last changed. Audio time, not page time: a late reading (a timer of a background tab, a long
 * task, the computer waking up) finds a new report when the engine ran meanwhile, and a clock that stands still (an
 * output that stopped pulling, a suspension) never makes an engine stall: that one is the output's. The first reading
 * with a new report clears it.
 */
import { STALL_TICKS } from './output-stall'

export { STALL_TICKS }
/** Audio clock, in seconds, that may run without a new report before the engine counts as stalled. */
export const STALL_CLOCK_S = 1

export interface ReportReading {
  /** The page's clock (`performance.now()`), in ms. */
  now: number
  /** The audio context's clock (`currentTime`), in seconds. */
  clock: number
  /** The report's `seq` as the engine last wrote it (odd mid-write), 0 before any: only its changes count. */
  seq: number
  /**
   * Whether the engine is expected to render: the context started and the page did not suspend it (Boost plays).
   * `false` forgets what was seen: the next readings start again.
   */
  expected: boolean
}

export class EngineStall {
  /** The report's `seq` and the audio clock at the reading where it last changed, and the page time of the last reading. */
  private last: { seq: number; clock: number; at: number } | null = null
  /** Readings in a row that found the same report. */
  private still = 0
  stalled = false

  /** Takes one reading in and returns whether the engine is stalled now. */
  observe({ now, clock, seq, expected }: ReportReading): boolean {
    if (!expected || !Number.isFinite(clock) || !Number.isFinite(now)) {
      this.last = null
      this.still = 0
      this.stalled = false
      return false
    }
    const last = this.last
    if (last === null || seq !== last.seq || now < last.at || clock < last.clock) {
      // A new report (or a clock that went back): the engine runs, and the next stall counts from here.
      this.last = { seq, clock, at: now }
      this.still = 0
      this.stalled = false
      return false
    }
    this.still++
    last.at = now
    if (this.still >= STALL_TICKS && clock - last.clock >= STALL_CLOCK_S) this.stalled = true
    return this.stalled
  }
}
