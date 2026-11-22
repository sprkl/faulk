import { describe, expect, it } from 'vitest'
import { ART_SIZE, artOf, hash, mix, rng, seedOf } from './art'
import { ENTRIES } from './entries'

const [a, b] = ENTRIES

describe('journey cover', () => {
  it('is deterministic: the same entry gives the same cover, call after call', () => {
    for (const e of ENTRIES) expect(artOf(structuredClone(e))).toEqual(artOf(e))
  })

  it('pins the seed function (FNV-1a) and the generator (mulberry32), so a cover never changes between builds', () => {
    expect(hash('')).toBe(0x811c9dc5)
    expect(hash('a')).toBe(0xe40c292c)
    const r = rng(42)
    const first = [r(), r(), r()]
    const again = rng(42)
    expect([again(), again(), again()]).toEqual(first)
    for (const x of first) expect(x >= 0 && x < 1).toBe(true)
  })

  it('differs between entries, and with the date or the music', () => {
    expect(seedOf(a)).not.toBe(seedOf(b))
    expect(artOf(a)).not.toEqual(artOf(b))
    expect(seedOf({ ...a, date: '2030-01-01' })).not.toBe(seedOf(a))
    expect(seedOf({ ...a, bpm: a.bpm + 1 })).not.toBe(seedOf(a))
    // The texts are not part of the seed: fixing a typo keeps the cover.
    expect(seedOf({ ...a, brief: { en: 'other', fr: 'autre' } })).toBe(seedOf(a))
  })

  it('draws from the data: one focus per track in its colour, the mode’s shape, rings by tempo, the form bar by bar', () => {
    for (const e of ENTRIES) {
      const art = artOf(e)
      expect(art.foci.map((f) => f.color)).toEqual(e.tracks.slice(0, 8).map((t) => t.color.toLowerCase()))
      expect(art.object.kind).toBe({ minor: 'orb', phrygian: 'orb', dorian: 'ring', mixolydian: 'ring', major: 'tile', lydian: 'tile' }[e.key.mode])
      expect(art.form).toHaveLength(e.sections.reduce((n, s) => n + s.bars * s.parts, 0))
      expect(new Set(art.form.map((d) => d.x)).size).toBe(e.bars)
      for (const p of [...art.foci, ...art.form]) {
        expect(p.x).toBeGreaterThanOrEqual(-ART_SIZE)
        expect(p.x).toBeLessThanOrEqual(ART_SIZE * 2)
      }
    }
    // Faster tempo, tighter rings.
    const gap = (bpm: number) => {
      const r = artOf({ ...a, bpm }).rings
      return r[1].r - r[0].r
    }
    expect(gap(140)).toBeLessThan(gap(80))
  })

  it('mixes colours', () => {
    expect(mix('#000000', '#ffffff', 0.5)).toBe('#808080')
    expect(mix('#ff0000', '#0000ff', 0)).toBe('#ff0000')
  })
})
