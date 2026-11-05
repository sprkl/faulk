/**
 * The spectra of the report (#156) by device id: what an equaliser's panel draws behind its curve.
 *
 * A spectrum names its device by slots (track slot, device slot) and by the serial of the engine's
 * instance. The page maps slots to ids with the slot table of its view (`ViewSlots.devices`), which
 * changes the moment a chain edit runs, a few milliseconds before the engine applies it: until then
 * the engine's reports still name the old instances by the old slots. So when the slots of a device
 * change (moved, inserted, its neighbour removed), its spectra are taken only from an instance
 * other than the one last seen at its new slots: every chain edit that moves a device builds a new
 * instance with a new serial (`DeviceEntry::serial`), so a spectrum of the device that sat there
 * before is never drawn on this one's card.
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

const slotKey = (track: number, device: number) => `${track}:${device}`

export interface SpectrumSlots {
  epoch: number
  devices?: Readonly<Record<string, readonly number[] | undefined>>
}

/** Follows the reports' spectra for each device id. Plain state: the caller decides when to read it. */
export class SpectrumTracker {
  /** The spectra shown, by device id. */
  readonly shown = new Map<string, Shown>()
  /** The slots each device had at the last report. */
  private readonly slotsOf = new Map<string, string>()
  /** For a device whose slots changed: the serial its spectra must not have (the previous instance there), until one comes. */
  private readonly refused = new Map<string, number | null>()
  /** The last serial a report gave each slot pair. */
  private readonly serialAt = new Map<string, number>()

  /** Takes the spectra of a report, against the view's slot table. */
  feed(epoch: number, spectra: readonly Spectrum[], slots: SpectrumSlots | undefined, now: number) {
    const byKey = new Map<string, string>()
    for (const [id, pair] of Object.entries(slots?.devices ?? {})) {
      if (!pair) continue
      const key = slotKey(pair[0], pair[1])
      byKey.set(key, id)
      const before = this.slotsOf.get(id)
      if (before !== key) {
        // A new place: refuse the instance last seen there (none seen: nothing to refuse).
        this.refused.set(id, this.serialAt.get(key) ?? null)
        this.slotsOf.set(id, key)
      }
    }
    // Devices no longer mapped are forgotten.
    for (const id of [...this.slotsOf.keys()]) {
      if (!slots?.devices?.[id]) {
        this.slotsOf.delete(id)
        this.refused.delete(id)
        this.shown.delete(id)
      }
    }
    if (!slots || epoch !== slots.epoch) return
    for (const s of spectra) {
      const key = slotKey(s.track, s.device)
      const id = byKey.get(key)
      const refused = id === undefined ? undefined : this.refused.get(id)
      if (id !== undefined && (refused === undefined || refused === null || s.serial !== refused)) {
        this.refused.delete(id)
        this.shown.set(id, { bands: s.bands, sampleRate: s.sampleRate, at: now })
      }
      this.serialAt.set(key, s.serial)
    }
  }

  /** The spectrum of a device, or `null` when it has none fresh at `now`. */
  get(id: string, now: number): Shown | null {
    const s = this.shown.get(id)
    return s && now - s.at <= STALE_MS ? s : null
  }
}
