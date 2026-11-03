/**
 * The Journey's entries: every file of `entries/`, checked, newest first. Adding a day is adding
 * one file there (and its folder in `sprkl/faulk-journey`); nothing else lists the entries.
 */

import { MODES, TONICS, type JourneyEntry, type Localized } from './types'

/** Where the audio, MIDI and sheets live (jsDelivr serves them with open CORS and the right types). */
export const JOURNEY_REPO = 'sprkl/faulk-journey'
const CDN = `https://cdn.jsdelivr.net/gh/${JOURNEY_REPO}@main/tracks`
const BLOB = `https://github.com/${JOURNEY_REPO}/blob/main/tracks`

/** The entry's folder in the journey repository, and its file stem in `entries/`. */
export function folderOf(entry: Pick<JourneyEntry, 'date' | 'slug'>): string {
  return `${entry.date}-${entry.slug}`
}

export function audioUrl(entry: JourneyEntry): string {
  return `${CDN}/${folderOf(entry)}/track.mp3`
}

export function midiUrl(entry: JourneyEntry): string {
  return `${CDN}/${folderOf(entry)}/track.mid`
}

/** The day's Faulk project, as the agent built it: Faulk opens it by this link (`/app?open=`, #497). */
export function packUrl(entry: JourneyEntry): string {
  return `${CDN}/${folderOf(entry)}/project.faulkpack`
}

/** The sheet on GitHub, where Markdown reads as a page. */
export function sheetUrl(entry: JourneyEntry): string {
  return `${BLOB}/${folderOf(entry)}/sheet.md`
}

const DATE = /^(\d{4})-(\d{2})-(\d{2})$/
const SLUG = /^[a-z0-9]+(?:-[a-z0-9]+)*$/
const COMMIT = /^[0-9a-f]{7,40}$/
const COLOR = /^#[0-9a-f]{6}$/i
/** A style profile's name: one line, 1 to 40 characters, no space around it. */
const STYLE = /^\S(?:[^\r\n\p{Zl}\p{Zp}]{0,38}\S)?$/u
const JOURNAL = /^https:\/\/github\.com\/sprkl\/faulk\/issues\/362#issuecomment-\d+$/

/** A real calendar day (`2026-02-30` is not). */
function isDay(value: string): boolean {
  const m = DATE.exec(value)
  if (!m) return false
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])))
  return d.toISOString().slice(0, 10) === value
}

function localizedErrors(value: unknown, what: string): string[] {
  const v = value as Partial<Localized> | undefined
  const errors: string[] = []
  for (const lang of ['en', 'fr'] as const) if (typeof v?.[lang] !== 'string' || !v[lang].trim()) errors.push(`${what}: missing ${lang} text`)
  return errors
}

/**
 * What is wrong with one entry, given the stem of its file (`2026-11-02-neon-sur-la-rocade`).
 * Empty when it is valid.
 */
export function entryErrors(entry: JourneyEntry, stem: string): string[] {
  const e: string[] = []
  if (!isDay(entry.date)) e.push(`date: "${entry.date}" is not a YYYY-MM-DD day`)
  if (!SLUG.test(entry.slug)) e.push(`slug: "${entry.slug}" must be lower-case ASCII words joined by -`)
  if (stem !== folderOf(entry)) e.push(`file: "${stem}" must be named after "${folderOf(entry)}"`)
  if (!entry.title?.trim()) e.push('title: empty')
  if (!COMMIT.test(entry.commit)) e.push(`commit: "${entry.commit}" is not a short hash`)
  e.push(...localizedErrors(entry.genre, 'genre'))
  if (!(Number.isFinite(entry.bpm) && entry.bpm >= 20 && entry.bpm <= 400)) e.push(`bpm: ${entry.bpm} out of 20..400`)
  if (!TONICS.includes(entry.key?.tonic)) e.push(`key.tonic: "${entry.key?.tonic}" unknown`)
  if (!MODES.includes(entry.key?.mode)) e.push(`key.mode: "${entry.key?.mode}" unknown`)
  if (!(Number.isInteger(entry.bars) && entry.bars > 0)) e.push(`bars: ${entry.bars} is not a positive whole number`)
  if (!(Number.isFinite(entry.duration) && entry.duration > 0 && entry.duration < 3600)) e.push(`duration: ${entry.duration} s out of range`)
  if (!entry.model?.trim()) e.push('model: empty')
  if (entry.style !== undefined && !STYLE.test(entry.style))
    e.push(`style: "${entry.style}" must be one line (no line break) of 1 to 40 characters, with no space around it`)
  if (!entry.skills?.length || entry.skills.some((s) => !s.trim())) e.push('skills: none, or an empty name')
  e.push(...localizedErrors(entry.brief, 'brief'), ...localizedErrors(entry.built, 'built'))
  if (!entry.tested?.length) e.push('tested: empty')
  entry.tested?.forEach((t, i) => e.push(...localizedErrors(t, `tested[${i}]`)))
  if (!entry.gaps?.length) e.push('gaps: empty')
  entry.gaps?.forEach((g, i) => e.push(...localizedErrors(g, `gaps[${i}]`)))
  if (!JOURNAL.test(entry.journal)) e.push(`journal: "${entry.journal}" is not a comment of issue #362`)
  if (!entry.tracks?.length) e.push('tracks: empty')
  entry.tracks?.forEach((t, i) => {
    if (!t.name?.trim()) e.push(`tracks[${i}]: empty name`)
    if (!COLOR.test(t.color)) e.push(`tracks[${i}]: colour "${t.color}" is not #rrggbb`)
  })
  if (!entry.sections?.length) e.push('sections: empty')
  entry.sections?.forEach((s, i) => {
    e.push(...localizedErrors(s.name, `sections[${i}].name`))
    if (!(Number.isInteger(s.bars) && s.bars > 0)) e.push(`sections[${i}]: bars ${s.bars}`)
    if (!(Number.isInteger(s.parts) && s.parts > 0)) e.push(`sections[${i}]: parts ${s.parts}`)
  })
  const sum = entry.sections?.reduce((n, s) => n + s.bars, 0) ?? 0
  if (entry.sections?.length && sum !== entry.bars) e.push(`sections: ${sum} bars, the entry says ${entry.bars}`)
  return e
}

/**
 * Checks every module of `entries/` (path → default export) and returns the entries, newest
 * first. Throws on the first invalid set, so a bad entry fails the build and the tests instead of
 * reaching the page.
 */
export function collect(modules: Record<string, { default: JourneyEntry }>): JourneyEntry[] {
  const errors: string[] = []
  const entries: JourneyEntry[] = []
  for (const [path, mod] of Object.entries(modules)) {
    const stem = path.replace(/^.*\//, '').replace(/\.ts$/, '')
    const entry = mod.default
    for (const err of entryErrors(entry, stem)) errors.push(`${stem}: ${err}`)
    entries.push(entry)
  }
  const days = entries.map((e) => e.date)
  for (const day of new Set(days)) if (days.filter((d) => d === day).length > 1) errors.push(`${day}: more than one entry`)
  if (errors.length) throw new Error(`Invalid journey entries:\n${errors.join('\n')}`)
  return entries.sort((a, b) => (a.date < b.date ? 1 : -1))
}

/** Every entry, newest first. */
export const ENTRIES: JourneyEntry[] = collect(import.meta.glob<{ default: JourneyEntry }>('./entries/*.ts', { eager: true }))

/** How many earlier days the main page lists under the featured one; the rest is in the monthly archives. */
export const RECENT = 10

/** `2026-11` from an entry. */
export function monthOf(entry: Pick<JourneyEntry, 'date'>): string {
  return entry.date.slice(0, 7)
}

/** The months that have entries, newest first. */
export function monthsOf(entries: JourneyEntry[]): string[] {
  return [...new Set(entries.map(monthOf))]
}

export const MONTHS: string[] = monthsOf(ENTRIES)

/** One month's entries, newest first. */
export function entriesOf(month: string, entries: JourneyEntry[] = ENTRIES): JourneyEntry[] {
  return entries.filter((e) => monthOf(e) === month)
}

/** The months around `month` that have entries: `older` before it, `newer` after it. */
export function neighbours(month: string, months: string[] = MONTHS): { older?: string; newer?: string } {
  const i = months.indexOf(month)
  if (i < 0) return {}
  return { newer: months[i - 1], older: months[i + 1] }
}

/** The entry whose page is `<date>-<slug>`. */
export function entryByFolder(folder: string, entries: JourneyEntry[] = ENTRIES): JourneyEntry | undefined {
  return entries.find((e) => folderOf(e) === folder)
}
