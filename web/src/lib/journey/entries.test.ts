import { readdirSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { audioUrl, collect, ENTRIES, entryErrors, folderOf, midiUrl, sheetUrl } from './entries'
import type { JourneyEntry } from './types'

const files = readdirSync(new URL('./entries/', import.meta.url)).filter((f) => f.endsWith('.ts'))

/** A valid entry to break one field at a time. */
const base = ENTRIES[ENTRIES.length - 1]
const stem = folderOf(base)
const broken = (patch: Partial<JourneyEntry>) => entryErrors({ ...base, ...patch }, stem)

describe('journey entries', () => {
  it('loads every file of entries/, each valid, newest first, one per day', () => {
    expect(files.length).toBeGreaterThanOrEqual(3)
    expect(ENTRIES).toHaveLength(files.length)
    for (const entry of ENTRIES) expect(entryErrors(entry, folderOf(entry)), folderOf(entry)).toEqual([])
    const days = ENTRIES.map((e) => e.date)
    expect([...days].sort().reverse()).toEqual(days)
    expect(new Set(days).size).toBe(days.length)
    expect(files.map((f) => f.replace(/\.ts$/, '')).sort()).toEqual(ENTRIES.map(folderOf).sort())
  })

  it('points at the journey repository through jsDelivr and GitHub', () => {
    const e = ENTRIES[0]
    expect(audioUrl(e)).toBe(`https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/${folderOf(e)}/track.mp3`)
    expect(midiUrl(e)).toBe(`https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/${folderOf(e)}/track.mid`)
    expect(sheetUrl(e)).toBe(`https://github.com/sprkl/faulk-journey/blob/main/tracks/${folderOf(e)}/sheet.md`)
  })

  it('refuses each kind of mistake, by name', () => {
    expect(broken({ date: '2026-02-30' })[0]).toMatch(/^date/)
    expect(broken({ date: '2026-2-3' }).some((e) => e.startsWith('date'))).toBe(true)
    expect(broken({ slug: 'Neon Rocade' }).some((e) => e.startsWith('slug'))).toBe(true)
    expect(entryErrors(base, '2026-01-01-other')).toEqual([`file: "2026-01-01-other" must be named after "${stem}"`])
    expect(broken({ title: ' ' })).toEqual(['title: empty'])
    expect(broken({ commit: 'develop' })[0]).toMatch(/^commit/)
    expect(broken({ bpm: 0 })[0]).toMatch(/^bpm/)
    expect(broken({ bpm: NaN })[0]).toMatch(/^bpm/)
    expect(broken({ key: { tonic: 'H' as never, mode: 'minor' } })[0]).toMatch(/^key.tonic/)
    expect(broken({ key: { tonic: 'A', mode: 'blues' as never } })[0]).toMatch(/^key.mode/)
    expect(broken({ duration: -1 })[0]).toMatch(/^duration/)
    expect(broken({ skills: [] })[0]).toMatch(/^skills/)
    expect(broken({ brief: { en: 'x', fr: '' } })).toEqual(['brief: missing fr text'])
    expect(broken({ gaps: [] })).toEqual(['gaps: empty'])
    expect(broken({ tested: [{ en: '', fr: 'x' }] })).toEqual(['tested[0]: missing en text'])
    expect(broken({ journal: 'https://github.com/sprkl/faulk/issues/361#issuecomment-1' })[0]).toMatch(/^journal/)
    expect(broken({ tracks: [{ name: 'Bass', color: 'red' }] })[0]).toMatch(/^tracks\[0\]: colour/)
    expect(broken({ sections: [{ name: { en: 'A', fr: 'A' }, bars: 3, parts: 1 }] })).toEqual([`sections: 3 bars, the entry says ${base.bars}`])
  })

  it('throws on a bad set: an invalid entry, or two entries on one day', () => {
    const mod = (e: JourneyEntry) => ({ default: e })
    expect(() => collect({ [`./entries/${stem}.ts`]: mod({ ...base, bpm: -5 }) })).toThrow(/bpm/)
    const twin = { ...base, slug: 'twin' }
    expect(() => collect({ [`./entries/${stem}.ts`]: mod(base), [`./entries/${folderOf(twin)}.ts`]: mod(twin) })).toThrow(/more than one entry/)
    expect(collect({ [`./entries/${stem}.ts`]: mod(base) })).toEqual([base])
  })
})
