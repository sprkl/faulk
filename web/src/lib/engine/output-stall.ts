/**
 * Tells when the audio output stopped pulling the Web engine (#810): the audio context's clock (`currentTime`), which
 * advances only as the output asks for audio, stood still while the page expects it to run. Nothing then plays, and the
 * engine's report (position, meters, load) stays frozen on its last values; the page says so instead.
 *
 * It runs on the main thread, from the page's feedback timer: it only compares readings of the clock. A stall needs
 * `STALL_TICKS` readings in a row with the clock unchanged, spanning `STALL_MS` of page time counted from the first of
 * them, not from the last reading where the clock moved: a late reading (a timer of a background tab, a long task, the
 * computer waking up while the audio clock did not run) is one reading, never a stall by itself, and an output that
 * takes a moment to come back after it is not one either. The first reading where the clock moved clears it.
 */

/** Page time with the clock unchanged before the output counts as stalled, in ms. */
export const STALL_MS = 1000
/** Readings in a row with the clock unchanged before the output counts as stalled. */
export const STALL_TICKS = 3

export interface ClockReading {
  /** The page's clock (`performance.now()`), in ms. */
  now: number
  /** The audio context's clock (`currentTime`), in seconds. */
  clock: number
  /**
   * Whether the output is expected to pull the engine: the context started and the page did not suspend it (Boost
   * plays). `false` forgets what was seen: the next readings start again.
   */
  expected: boolean
}

export class OutputStall {
  /** The clock of the last reading, and the page time of the first reading that found it unchanged (`null`: none yet). */
  private last: { clock: number; at: number; since: number | null } | null = null
  /** Readings in a row with the clock unchanged. */
  private still = 0
  stalled = false

  /** Takes one reading in and returns whether the output is stalled now. */
  observe({ now, clock, expected }: ClockReading): boolean {
    if (!expected || !Number.isFinite(clock) || !Number.isFinite(now)) {
      this.last = null
      this.still = 0
      this.stalled = false
      return false
    }
    if (this.last === null || clock !== this.last.clock || now < this.last.at) {
      // The clock moved (or the page clock went back): the output runs, and the next stall counts from here.
      this.last = { clock, at: now, since: null }
      this.still = 0
      this.stalled = false
      return false
    }
    this.still++
    this.last.at = now
    this.last.since ??= now
    if (this.still >= STALL_TICKS && now - this.last.since >= STALL_MS) this.stalled = true
    return this.stalled
  }
}
