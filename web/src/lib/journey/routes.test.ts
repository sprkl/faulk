import { describe, expect, it } from 'vitest'
import { params } from '../../params'
import { entries as langDays } from '../../routes/[lang=lang]/journey/[day=day]/+page'
import { entries as langMonths } from '../../routes/[lang=lang]/journey/[month=month]/+page'
import { entries as days } from '../../routes/journey/[day=day]/+page'
import { entries as months } from '../../routes/journey/[month=month]/+page'
import { ENTRIES, entriesOf, entryByFolder, folderOf, monthOf, monthsOf, MONTHS, neighbours, RECENT } from './entries'

// The routes' `entries()` decide what is prerendered: every day and every month must be in them,
// in each language and without a prefix, and accepted by the route's matcher.
describe('journey routes', () => {
  const allDays = ENTRIES.map(folderOf).sort()
  const allMonths = [...new Set(ENTRIES.map(monthOf))].sort()

  it('prerender every day and every month without a prefix', async () => {
    expect((await days()).map((p) => p.day).sort()).toEqual(allDays)
    expect((await months()).map((p) => p.month).sort()).toEqual(allMonths)
  })

  it('prerender every day and every month in English and French', async () => {
    for (const lang of ['en', 'fr']) {
      expect(
        (await langDays())
          .filter((p) => p.lang === lang)
          .map((p) => p.day)
          .sort(),
      ).toEqual(allDays)
      expect(
        (await langMonths())
          .filter((p) => p.lang === lang)
          .map((p) => p.month)
          .sort(),
      ).toEqual(allMonths)
    }
  })

  it('match every day and month with their parameter matchers, and nothing else', () => {
    type Matcher = { '~standard': { validate(v: unknown): { value?: unknown; issues?: unknown } } }
    const match = (name: 'day' | 'month', value: string) => (params[name] as unknown as Matcher)['~standard'].validate(value).value
    for (const d of allDays) expect(match('day', d)).toBe(d)
    for (const m of allMonths) expect(match('month', m)).toBe(m)
    expect(match('month', '2026-11-02')).toBeUndefined()
    expect(match('day', '2026-11')).toBeUndefined()
    expect(match('day', '2026-11-02-Neon')).toBeUndefined()
  })
})

describe('journey months', () => {
  it('lists the months newest first, each with its entries newest first', () => {
    expect(MONTHS).toEqual([...allMonthsDesc()])
    for (const m of MONTHS) {
      const list = entriesOf(m)
      expect(list.length).toBeGreaterThan(0)
      expect(list.every((e) => monthOf(e) === m)).toBe(true)
      expect(list.map((e) => e.date)).toEqual([...list.map((e) => e.date)].sort().reverse())
    }
    expect(MONTHS.reduce((n, m) => n + entriesOf(m).length, 0)).toBe(ENTRIES.length)
  })

  it('links each month to the one before and after', () => {
    const list = ['2026-12', '2026-11', '2026-09']
    expect(neighbours('2026-12', list)).toEqual({ newer: undefined, older: '2026-11' })
    expect(neighbours('2026-11', list)).toEqual({ newer: '2026-12', older: '2026-09' })
    expect(neighbours('2026-09', list)).toEqual({ newer: '2026-11', older: undefined })
    expect(neighbours('2025-01', list)).toEqual({})
  })

  it('finds a day by its folder, and keeps the main page to the latest day plus RECENT', () => {
    for (const e of ENTRIES) expect(entryByFolder(folderOf(e))).toBe(e)
    expect(entryByFolder('2026-01-01-nothing')).toBeUndefined()
    expect(RECENT).toBe(10)
    // A year of days: the months group them, newest first.
    const year = Array.from({ length: 365 }, (_, i) => {
      const d = new Date(Date.UTC(2027, 0, 1 + i)).toISOString().slice(0, 10)
      return { ...ENTRIES[0], date: d, slug: 'x' }
    }).reverse()
    expect(monthsOf(year)).toHaveLength(12)
    expect(monthsOf(year)[0]).toBe('2027-12')
  })
})

function allMonthsDesc(): string[] {
  return [...new Set(ENTRIES.map(monthOf))].sort().reverse()
}
