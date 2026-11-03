// The engine's CPU load as the pill and the dialog show it (#486): the reports carry the load of
// each period (about 1/30 s) and its peak; the page smooths the one and holds the other, the same
// way for both modes.

/** Time constant of the smoothed load, in milliseconds. */
export const LOAD_SMOOTHING_MS = 1000
/** How long a peak stays shown, in milliseconds. */
export const PEAK_HOLD_MS = 3000

/** The load shown: smoothed and the peak held, 0 to 1. */
export interface EngineLoad {
  smoothed: number
  peak: number
}

/**
 * Smooths the reports' load (an exponential moving average whose weight follows the real time
 * between two reports, so that a dropped or late report weighs right) and holds the largest
 * peak of the last [`PEAK_HOLD_MS`].
 */
export class LoadMeter {
  private smoothed: number | null = null
  private last = 0
  private peaks: { at: number; peak: number }[] = []

  /** Takes one report's load and peak at `now` (ms); `null` while the host does not measure. */
  update(load: number | null, peak: number | null, now: number): EngineLoad | null {
    if (load == null) {
      this.reset()
      return null
    }
    const dt = Math.max(0, now - this.last)
    this.smoothed = this.smoothed == null ? load : this.smoothed + (load - this.smoothed) * (1 - Math.exp(-dt / LOAD_SMOOTHING_MS))
    this.last = now
    this.peaks.push({ at: now, peak: Math.max(peak ?? load, 0) })
    while (this.peaks.length > 0 && now - this.peaks[0].at > PEAK_HOLD_MS) this.peaks.shift()
    let held = 0
    for (const p of this.peaks) held = Math.max(held, p.peak)
    return { smoothed: this.smoothed, peak: Math.max(held, this.smoothed) }
  }

  /** Forgets everything: a new engine starts from its first report. */
  reset() {
    this.smoothed = null
    this.peaks = []
  }
}
