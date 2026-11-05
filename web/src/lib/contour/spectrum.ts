/**
 * The spectra of the report (#156) by device id: what an equaliser's panel draws behind its curve.
 *
 * A spectrum names its device by slots (track slot, device slot). The page maps slots to ids with
 * the slot table of its view (`ViewSlots.devices`), which changes the moment a chain edit runs; the
 * engine applies the edit a few milliseconds later, and a spectrum comes one report after its
 * window: for a few reports, the reports still name the devices by their old slots. So once the
 * slots of a device change (moved, inserted, a neighbour removed), its spectra are left out for
 * `SETTLE_MS` (about six reports), and a spectrum of the device that sat there before is never drawn
 * on this one's card. A device that keeps its slots is never held back.
 */

import type { Spectrum } from '../engine/feedback'

/** A spectrum the page draws: its bands in dBFS and the rate they were measured at. */
export interface Shown {
  bands: Float32Array
  sampleRate: number
  /** The page's clock (ms) when it arrived. */
  at: number
}

/** After this long without a new spectrum, a device shows none (bypassed, removed, the engine stopped). */
export const STALE_MS = 400
/** How long the spectra of a device whose slots changed are left out, in ms. */
export const SETTLE_MS = 200

const slotKey = (track: number, device: number) => `${track}:${device}`

export interface SpectrumSlots {
  epoch: number
  devices?: Readonly<Record<string, readonly number[] | undefined>>
}

/** Follows the reports' spectra for each device id. Plain state: the caller decides when to read it. */
export class SpectrumTracker {
  /** The spectra shown, by device id. */
  readonly shown = new Map<string, Shown>()
  /** The slots each device had at the last report, and since when. */
  private readonly slotsOf = new Map<string, { key: string; since: number }>()

  /** Takes the spectra of a report, against the view's slot table, at the page's clock `now`. */
  feed(epoch: number, spectra: readonly Spectrum[], slots: SpectrumSlots | undefined, now: number) {
    const byKey = new Map<string, string>()
    const devices = slots?.devices ?? {}
    for (const [id, pair] of Object.entries(devices)) {
      if (!pair) continue
      const key = slotKey(pair[0], pair[1])
      byKey.set(key, id)
      const before = this.slotsOf.get(id)
      // A device seen for the first time is new at its slots too: whatever sat there before may still be reported.
      if (before?.key !== key) {
        this.slotsOf.set(id, { key, since: now })
        this.shown.delete(id)
      }
    }
    // Devices no longer mapped are forgotten.
    for (const id of [...this.slotsOf.keys()]) {
      if (!devices[id]) {
        this.slotsOf.delete(id)
        this.shown.delete(id)
      }
    }
    if (!slots || epoch !== slots.epoch) return
    for (const s of spectra) {
      const id = byKey.get(slotKey(s.track, s.device))
      if (id === undefined) continue
      const place = this.slotsOf.get(id)
      if (place && now - place.since < SETTLE_MS) continue
      this.shown.set(id, { bands: s.bands, sampleRate: s.sampleRate, at: now })
    }
  }

  /** The spectrum of a device, or `null` when it has none fresh at `now`. */
  get(id: string, now: number): Shown | null {
    const s = this.shown.get(id)
    return s && now - s.at <= STALE_MS ? s : null
  }
}
