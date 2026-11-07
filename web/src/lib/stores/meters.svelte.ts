import { untrack } from 'svelte'
import { MeterBank, SILENT, type MeterSnap } from '../meters/ballistics'
import type { EngineStore } from './engine.svelte'
import type { ProjectStore } from './project.svelte'

/** The master's id in the store (a track id is a UUID: they never meet). */
export const MASTER_METER = 'master'

/** Reports stop when the engine stops or the helper goes: after this long without one, the meters fall to silence. */
const STALE_MS = 500

/**
 * What every meter of the page shows (lane headers, mixer strips, the top bar's master, the
 * instrument view), kept in one place so they agree: the engine's report (30 Hz) feeds one
 * `MeterBank`, a latched CLIP is reset for the strip wherever it is clicked, and the meters
 * survive a change of view. A track's meter reads the meter of its slot (`ViewSlots`), only when
 * the report is of the slot table's epoch; anything else is silence.
 */
export class MeterStore {
  /** The snapshot of each strip, by track id (and `MASTER_METER`), replaced as a whole at each report. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  snaps = $state.raw<Map<string, MeterSnap>>(new Map())

  private readonly bank = new MeterBank()
  private lastReportAt = 0
  /** The report the strips were last fed from, to feed them again when the tracks change. */
  private lastReport: NonNullable<EngineStore['feedback']> | null = null
  /** The last report fed: one that comes back (the engine restarted, no new report yet) is old news, not a fresh signal. */
  private seen: object | null = null

  constructor(
    private readonly project: ProjectStore,
    private readonly engine: EngineStore,
  ) {}

  /** A strip's snapshot; silence for a strip with none. */
  snap(id: string): MeterSnap {
    return this.snaps.get(id) ?? SILENT
  }

  /** Clears one strip's peak readout and clip mark. */
  reset(id: string) {
    // A copy that replaces the raw state, never mutated in place.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const next = new Map(this.snaps)
    next.set(id, this.bank.reset(id))
    this.snaps = next
  }

  /**
   * The live value of a parameter a device moves by itself (#686: the Filter's cutoff under its LFO), by device id
   * then parameter id, from the last report of the slot table's epoch; empty when reports stop.
   */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  readouts = $state.raw<Map<string, Map<number, number>>>(new Map())

  /** The live value of a device's parameter, or `undefined` when the engine reports none. */
  readout(device: string, param: number): number | undefined {
    return this.readouts.get(device)?.get(param)
  }

  /** Feeds the bank from a report, or silence (`null`). Reads nothing reactive. */
  feed(report: NonNullable<EngineStore['feedback']> | null, now: number) {
    const slots = this.project.view?.slots
    // A copy that replaces the raw state, never mutated in place.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const readouts = new Map<string, Map<number, number>>()
    if (report && slots && report.epoch === slots.epoch && report.readouts.length > 0) {
      // eslint-disable-next-line svelte/prefer-svelte-reactivity
      const byPlace = new Map<string, string>()
      for (const [id, place] of Object.entries(slots.devices)) if (place) byPlace.set(`${place.track ?? 'm'}:${place.slot}`, id)
      for (const r of report.readouts) {
        const id = byPlace.get(`${r.track ?? 'm'}:${r.device}`)
        if (id === undefined) continue
        // eslint-disable-next-line svelte/prefer-svelte-reactivity
        const params = readouts.get(id) ?? new Map<number, number>()
        params.set(r.param, r.value)
        readouts.set(id, params)
      }
    }
    if (readouts.size > 0 || this.readouts.size > 0) this.readouts = readouts
    const ids = this.project.tracks.map((entry) => entry.id)
    const silence = { peakL: 0, peakR: 0 }
    this.snaps = this.bank.update(
      [...ids, MASTER_METER],
      (id) => {
        if (!report) return silence
        if (id === MASTER_METER) return report.master
        const slot = slots?.tracks[id]
        // A slot beyond the report (a track added since, an engine a little behind) is silent, not an error.
        return slot === undefined || report.epoch !== slots?.epoch ? silence : (report.tracks[slot] ?? silence)
      },
      now / 1000,
    )
  }

  /** Follows the engine's reports until the returned function is called. Call it from a component. */
  start(): () => void {
    return $effect.root(() => {
      // One pass per new report. A project change in between must not feed the same peaks twice, so only the report is tracked.
      // The clock is the page's, taken when the report arrives: a report carries none.
      $effect(() => {
        const current = this.engine.started ? this.engine.feedback : null
        const now = performance.now()
        untrack(() => {
          const report = current !== null && current === this.seen ? null : current
          if (current) this.seen = current
          if (report) {
            this.lastReportAt = now
            this.lastReport = report
          }
          this.feed(report, now)
        })
      })
      // A track added or removed: the strips follow the tracks at once, from the last report, never from silence.
      $effect(() => {
        void this.project.tracks
        untrack(() => {
          if (this.lastReport && performance.now() - this.lastReportAt <= STALE_MS) this.feed(this.lastReport, performance.now())
        })
      })
      $effect(() => {
        const timer = setInterval(() => {
          const now = performance.now()
          if (now - this.lastReportAt > STALE_MS) this.feed(null, now)
        }, 100)
        return () => clearInterval(timer)
      })
    })
  }
}
