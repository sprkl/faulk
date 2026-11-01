/**
 * What a meter shows from the engine's peaks: the bar (instant attack, steady release), a hold
 * marker that stays at the last peak for a moment, and the highest peak since the last reset
 * (the readout under the meter). Pure: the caller gives the time of each report (the page's clock
 * at its arrival, a report carries none), so a replay in a test is exact.
 */

/** The meter's floor and ceiling, in dB. */
export const METER_FLOOR_DB = -60
export const METER_CEIL_DB = 6
/** At or above this peak (linear) the signal is at full scale: the clip mark. */
export const CLIP_PEAK = 0.999
/** How fast the bar falls, in dB per second. */
export const RELEASE_DB_PER_S = 24
/** How long the hold marker stays at a peak, in seconds, then how fast it falls (dB per second). */
export const HOLD_S = 1.5
export const HOLD_FALL_DB_PER_S = 12
/** The longest gap between two reports that counts: a longer one (a hidden tab) is clamped. */
const MAX_STEP_S = 0.25

/** A linear peak in dB, floored; silence and anything not finite are the floor. */
export function toDb(peak: number): number {
  return peak > 0 && Number.isFinite(peak) ? Math.max(METER_FLOOR_DB, 20 * Math.log10(peak)) : METER_FLOOR_DB
}

/** The share of the meter's height (0..1) a level in dB fills. */
export function fillOf(db: number): number {
  return Math.min(1, Math.max(0, (db - METER_FLOOR_DB) / (METER_CEIL_DB - METER_FLOOR_DB)))
}

export class Ballistics {
  /** The bar, in dB. */
  level = METER_FLOOR_DB
  /** The hold marker, in dB. */
  hold = METER_FLOOR_DB
  /** The highest peak since the last reset, in dB. */
  peak = METER_FLOOR_DB
  /** Whether the signal reached full scale since the last reset. */
  clipped = false
  private holdLeft = 0
  private at: number | null = null

  /** Takes a peak (linear) reported at `now` (seconds, any origin; the clock only needs to be monotonic). */
  update(peak: number, now: number) {
    const dt = this.at === null ? 0 : Math.min(MAX_STEP_S, Math.max(0, now - this.at))
    this.at = now
    const db = toDb(peak)
    this.level = Math.max(db, this.level - RELEASE_DB_PER_S * dt)
    if (db >= this.hold) {
      this.hold = db
      this.holdLeft = HOLD_S
    } else if (this.holdLeft > 0) {
      this.holdLeft -= dt
    } else {
      this.hold = Math.max(db, METER_FLOOR_DB, this.hold - HOLD_FALL_DB_PER_S * dt)
    }
    if (db > this.peak) this.peak = db
    if (peak >= CLIP_PEAK) this.clipped = true
  }

  /** Clears the readout and the clip mark (a click on the meter). */
  reset() {
    this.peak = METER_FLOOR_DB
    this.clipped = false
  }
}

/** What a stereo meter draws: fills are shares of the meter's height (0..1). */
export interface MeterSnap {
  levelL: number
  levelR: number
  holdL: number
  holdR: number
  /** The highest peak since the last reset, in dB (the floor when none). */
  peakDb: number
  clipped: boolean
  /** The louder bar, in dB, for the meter's accessible value. */
  levelDb: number
  /** The largest gain reduction of the strip's compressors, in dB (>= 0; 0 when none works or the engine reports none). */
  reductionDb: number
}

/** The snapshot of a silent meter. */
export const SILENT: MeterSnap = { levelL: 0, levelR: 0, holdL: 0, holdR: 0, peakDb: METER_FLOOR_DB, clipped: false, levelDb: METER_FLOOR_DB, reductionDb: 0 }

class StereoMeter {
  readonly l = new Ballistics()
  readonly r = new Ballistics()

  /** The strip's gain reduction in the last report, in dB. */
  reductionDb = 0

  snap(): MeterSnap {
    return {
      reductionDb: this.reductionDb,
      levelL: fillOf(this.l.level),
      levelR: fillOf(this.r.level),
      holdL: fillOf(this.l.hold),
      holdR: fillOf(this.r.hold),
      peakDb: Math.max(this.l.peak, this.r.peak),
      clipped: this.l.clipped || this.r.clipped,
      levelDb: Math.max(this.l.level, this.r.level),
    }
  }
}

/**
 * The meters of the mixer, by strip id. A report feeds every strip it lists and drops the others,
 * so a deleted track leaves nothing behind: the bank never holds more than the strips on screen.
 */
export class MeterBank {
  private readonly meters = new Map<string, StereoMeter>()

  get size(): number {
    return this.meters.size
  }

  /** Feeds one report: `read` gives a strip's peaks (zeros for a strip the report does not cover). */
  update(ids: readonly string[], read: (id: string) => { peakL: number; peakR: number; gainReductionDb?: number }, now: number): Map<string, MeterSnap> {
    const next = new Map<string, StereoMeter>()
    const snaps = new Map<string, MeterSnap>()
    for (const id of ids) {
      const meter = this.meters.get(id) ?? new StereoMeter()
      const { peakL, peakR, gainReductionDb } = read(id)
      meter.reductionDb = gainReductionDb !== undefined && Number.isFinite(gainReductionDb) ? Math.max(0, gainReductionDb) : 0
      meter.l.update(peakL, now)
      meter.r.update(peakR, now)
      next.set(id, meter)
      snaps.set(id, meter.snap())
    }
    this.meters.clear()
    for (const [id, meter] of next) this.meters.set(id, meter)
    return snaps
  }

  /** Clears one strip's peak readout and clip mark; its snapshot, or the silent one for an unknown id. */
  reset(id: string): MeterSnap {
    const meter = this.meters.get(id)
    if (!meter) return SILENT
    meter.l.reset()
    meter.r.reset()
    return meter.snap()
  }
}
