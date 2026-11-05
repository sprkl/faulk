import { describe, expect, it } from 'vitest'
import type { Spectrum } from '../engine/feedback'
import { STALE_MS, SpectrumTracker } from './spectrum'

const spectrum = (track: number, device: number, serial: number, level = -20): Spectrum => ({
  track,
  device,
  serial,
  sampleRate: 48000,
  bands: new Float32Array(96).fill(level),
})

const slots = (devices: Record<string, [number, number]>, epoch = 1) => ({ epoch, devices })

describe('the spectra by device', () => {
  it('takes a spectrum by its slots, in the slot table epoch only, and drops it once stale', () => {
    const t = new SpectrumTracker()
    t.feed(1, [spectrum(0, 1, 5)], slots({ a: [0, 1] }), 0)
    expect(t.get('a', 0)?.bands[0]).toBe(-20)
    // Another epoch names other slots: not read.
    t.feed(2, [spectrum(0, 1, 5, -3)], slots({ a: [0, 1] }), 10)
    expect(t.get('a', 10)?.bands[0]).toBe(-20)
    // Nothing new for a while: none.
    expect(t.get('a', STALE_MS + 1)).toBeNull()
    // A slot the table does not map: nobody's.
    t.feed(1, [spectrum(3, 3, 9)], slots({ a: [0, 1] }), 20)
    expect(t.get('a', 20)?.bands[0]).toBe(-20)
  })

  it('never draws the previous instance of a slot on the device that moved there', () => {
    const t = new SpectrumTracker()
    // A at slot 1 (serial 5), B at slot 2 (serial 7).
    t.feed(1, [spectrum(0, 1, 5, -10), spectrum(0, 2, 7, -30)], slots({ a: [0, 1], b: [0, 2] }), 0)
    expect(t.get('a', 0)?.bands[0]).toBe(-10)
    expect(t.get('b', 0)?.bands[0]).toBe(-30)
    // B moved before A: the page's table swaps at once, the engine's report is still the old one.
    t.feed(1, [spectrum(0, 1, 5, -10), spectrum(0, 2, 7, -30)], slots({ a: [0, 2], b: [0, 1] }), 33)
    // Neither took the other's spectrum: each keeps its own last one.
    expect(t.get('a', 33)?.bands[0]).toBe(-10)
    expect(t.get('b', 33)?.bands[0]).toBe(-30)
    // The engine rebuilt both on their new slots (new serials): taken.
    t.feed(1, [spectrum(0, 1, 8, -31), spectrum(0, 2, 9, -11)], slots({ a: [0, 2], b: [0, 1] }), 66)
    expect(t.get('a', 66)?.bands[0]).toBe(-11)
    expect(t.get('b', 66)?.bands[0]).toBe(-31)
    // Later spectra of the same instances keep coming.
    t.feed(1, [spectrum(0, 1, 8, -32)], slots({ a: [0, 2], b: [0, 1] }), 99)
    expect(t.get('b', 99)?.bands[0]).toBe(-32)
  })

  it('takes any instance at a slot nothing was seen at, and forgets a removed device', () => {
    const t = new SpectrumTracker()
    t.feed(1, [], slots({ a: [0, 1] }), 0)
    t.feed(1, [spectrum(0, 1, 3)], slots({ a: [0, 1] }), 10)
    expect(t.get('a', 10)).not.toBeNull()
    t.feed(1, [spectrum(0, 1, 3)], slots({}), 20)
    expect(t.get('a', 20)).toBeNull()
    // Inserted again at the same slot: the old instance (serial 3) is refused, a new one taken.
    t.feed(1, [spectrum(0, 1, 3)], slots({ a: [0, 1] }), 30)
    expect(t.get('a', 30)).toBeNull()
    t.feed(1, [spectrum(0, 1, 4)], slots({ a: [0, 1] }), 40)
    expect(t.get('a', 40)).not.toBeNull()
  })

  it('reads the master chain by its track slot', () => {
    const t = new SpectrumTracker()
    t.feed(1, [spectrum(4294967295, 0, 2)], slots({ m: [4294967295, 0] }), 0)
    expect(t.get('m', 0)).not.toBeNull()
  })
})
