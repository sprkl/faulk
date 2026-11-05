import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import type { Note } from '../../lib/generated/protocol'
import { defaultRepeatUntil, parseRepeatUntil, repeatCopies, repeatPeriod } from './repeat'

type Raw = [number, number, number, number]
interface Case {
  name: string
  clipLength: number
  notes: Raw[]
  others: Raw[]
  grid: number | null
  until: number
  copies?: Raw[]
  added?: number
  atClipEnd?: boolean
  refused?: string
}

const note = ([start, pitch, length, velocity]: Raw): Note => ({ start, pitch, length, velocity })
const key = (n: Note) => `${n.start}:${n.pitch}:${n.length}:${n.velocity}`

// The table the command runs too (`crates/commands/tests/it/clip_edit.rs`): the page selects what it computes here.
const table = JSON.parse(readFileSync(fileURLToPath(new URL('../../../../crates/commands/tests/it/notes_repeat_cases.json', import.meta.url)), 'utf8')) as {
  cases: Case[]
}

describe('repeat end to end (#710), the cases shared with clip.notes.repeat', () => {
  it('reads the shared table', () => {
    expect(table.cases.length).toBeGreaterThanOrEqual(10)
  })
  for (const c of table.cases) {
    it(c.name, () => {
      const result = repeatCopies(c.notes.map(note), c.grid, c.until, c.clipLength)
      if (c.refused) {
        expect(result.ok).toBe(false)
        return
      }
      expect(result.ok).toBe(true)
      if (!result.ok) return
      expect(result.copies.map(key).sort()).toEqual(c.copies!.map(note).map(key).sort())
      expect(result.atClipEnd).toBe(c.atClipEnd)
      // What the command counts as added: the copies that were not in the clip.
      const there = new Set([...c.notes, ...c.others].map(note).map(key))
      expect(result.copies.filter((n) => !there.has(key(n))).length).toBe(c.added)
    })
  }
})

describe('repeat end to end (#710), the page side', () => {
  const figure = [note([0, 65, 240, 100]), note([240, 68, 240, 100]), note([480, 72, 240, 100]), note([720, 77, 240, 100])]
  const BAR = 3840

  it('says why it refuses', () => {
    expect(repeatCopies(figure, 0, BAR, BAR)).toMatchObject({ ok: false, refusal: 'grid' })
    expect(repeatCopies(figure, 240, 960, BAR)).toMatchObject({ ok: false, refusal: 'until', firstCopy: 960 })
    expect(repeatCopies([note([0, 60, BAR, 100])], 240, 2 * BAR, BAR)).toMatchObject({ ok: false, refusal: 'room', firstCopy: BAR })
    expect(repeatCopies([], 240, BAR, BAR)).toMatchObject({ ok: true, copies: [] })
  })

  it('rounds the span up to the grid', () => {
    expect(repeatPeriod(figure, 240)).toBe(960)
    expect(repeatPeriod([note([0, 60, 200, 100])], 240)).toBe(240)
    expect(repeatPeriod([note([0, 60, 200, 100])], null)).toBe(200)
    expect(repeatPeriod([note([0, 60, 900, 100])], 960 / 3)).toBe(960)
  })

  it('stops by default at the end of the bar where the copies begin', () => {
    expect(defaultRepeatUntil(figure, 240, BAR)).toBe(BAR)
    // A figure on the last beat, or a whole bar: the copies begin in the next bar and fill it.
    expect(defaultRepeatUntil([note([2880, 60, 960, 100])], 240, BAR)).toBe(2 * BAR)
    expect(defaultRepeatUntil([note([0, 60, BAR, 100])], 240, BAR)).toBe(2 * BAR)
    // A figure in bar 2 fills bar 2.
    expect(defaultRepeatUntil([note([BAR, 60, 240, 100])], 240, BAR)).toBe(2 * BAR)
  })

  it('reads a bar, or a bar and a beat, as the end of it', () => {
    const sig = { numerator: 4, denominator: 4 }
    expect(parseRepeatUntil('2', sig)).toBe(2 * BAR)
    expect(parseRepeatUntil(' 1.2 ', sig)).toBe(1920)
    expect(parseRepeatUntil('3.4', sig)).toBe(3 * BAR)
    expect(parseRepeatUntil('2.1', { numerator: 6, denominator: 8 })).toBe(2880 + 480)
    // A comma (a French keypad's decimal key) or a colon, as "Go to" reads them.
    expect(parseRepeatUntil('1,2', sig)).toBe(1920)
    expect(parseRepeatUntil('1:2', sig)).toBe(1920)
    for (const bad of ['', '0', '1.0', '1.5', 'x', '1.', '.2', '-1', '1.2.3', '1;2']) expect(parseRepeatUntil(bad, sig), bad).toBeNull()
  })
})
