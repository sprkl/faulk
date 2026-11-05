import { describe, expect, it } from 'vitest'
import type { Spectrum } from '../engine/feedback'
import { SETTLE_MS, STALE_MS, SpectrumTracker } from './spectrum'

const spectrum = (track: number, device: number, level = -20): Spectrum => ({
  track,
  device,
  sampleRate: 48000,
  bands: new Float32Array(96).fill(level),
})

const slots = (devices: Record<string, [number, number]>, epoch = 1) => ({ epoch, devices })

/** A tracker that has seen `devices` at their slots long enough to take their spectra. */
function settled(devices: Record<string, [number, number]>) {
  const t = new SpectrumTracker()
  t.feed(1, [], slots(devices), 0)
  return t
}

describe('the spectra by device', () => {
  it('takes a spectrum by its slots, in the slot table epoch only, and drops it once stale', () => {
    const t = settled({ a: [0, 1] })
    const at = SETTLE_MS
    t.feed(1, [spectrum(0, 1)], slots({ a: [0, 1] }), at)
    expect(t.get('a', at)?.bands[0]).toBe(-20)
    // Another epoch names other slots: not read.
    t.feed(2, [spectrum(0, 1, -3)], slots({ a: [0, 1] }), at + 10)
    expect(t.get('a', at + 10)?.bands[0]).toBe(-20)
    // Nothing new for a while: none.
    expect(t.get('a', at + STALE_MS + 1)).toBeNull()
    // A slot the table does not map: nobody's.
    t.feed(1, [spectrum(3, 3)], slots({ a: [0, 1] }), at + 20)
    expect(t.get('a', at + 20)?.bands[0]).toBe(-20)
  })

  it('never draws the device that sat at a slot on the one that moved there, and takes it again once settled', () => {
    const t = settled({ a: [0, 1], b: [0, 2] })
    let now = SETTLE_MS
    t.feed(1, [spectrum(0, 1, -10), spectrum(0, 2, -30)], slots({ a: [0, 1], b: [0, 2] }), now)
    expect(t.get('a', now)?.bands[0]).toBe(-10)
    expect(t.get('b', now)?.bands[0]).toBe(-30)
    // B moved before A: the page's table swaps at once, the engine's reports still name the old slots.
    now += 33
    t.feed(1, [spectrum(0, 1, -10), spectrum(0, 2, -30)], slots({ a: [0, 2], b: [0, 1] }), now)
    expect(t.get('a', now)).toBeNull()
    expect(t.get('b', now)).toBeNull()
    // Still within the settling time: the late reports are left out.
    now += SETTLE_MS - 40
    t.feed(1, [spectrum(0, 1, -10), spectrum(0, 2, -30)], slots({ a: [0, 2], b: [0, 1] }), now)
    expect(t.get('a', now)).toBeNull()
    // Settled: the engine names them by their new slots.
    now += 40
    t.feed(1, [spectrum(0, 1, -31), spectrum(0, 2, -11)], slots({ a: [0, 2], b: [0, 1] }), now)
    expect(t.get('a', now)?.bands[0]).toBe(-11)
    expect(t.get('b', now)?.bands[0]).toBe(-31)
  })

  it('holds back a new device until it settles, and forgets a removed one', () => {
    const t = new SpectrumTracker()
    t.feed(1, [spectrum(0, 1)], slots({ a: [0, 1] }), 0)
    expect(t.get('a', 0)).toBeNull()
    t.feed(1, [spectrum(0, 1)], slots({ a: [0, 1] }), SETTLE_MS)
    expect(t.get('a', SETTLE_MS)).not.toBeNull()
    t.feed(1, [spectrum(0, 1)], slots({}), SETTLE_MS + 10)
    expect(t.get('a', SETTLE_MS + 10)).toBeNull()
  })

  it('reads the master chain by its track slot', () => {
    const t = settled({ m: [4294967295, 0] })
    t.feed(1, [spectrum(4294967295, 0)], slots({ m: [4294967295, 0] }), SETTLE_MS)
    expect(t.get('m', SETTLE_MS)).not.toBeNull()
  })
})
