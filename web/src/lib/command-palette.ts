/**
 * The command palette (#108): the pure part. It lists the commands of the catalogue (`COMMAND_CATALOG`, generated from the
 * Rust catalogue: every `edit`, `live` and `setting` command, never a hand-written list), finds them by what is typed, and
 * turns what is typed into a field into a `Command` the dispatcher runs, like every other action. It owns no path of its own.
 */
import { COMMAND_CATALOG, type CatalogEntry, type Command, type CommandField } from './generated/protocol'
import type { MessageKey } from './i18n'
import { messages } from './i18n/app'
import { COMMAND_LABEL_PREFIX } from './i18n/history'

/** The key of a command's own palette label (`palette.cmd.<name>`), for the live commands the history never shows. */
export const PALETTE_LABEL_PREFIX = 'palette.cmd.'

/** The most results the palette shows at once: the list stays short enough to read, and the rest is found by typing more. */
export const MAX_RESULTS = 50

/** A command as the palette shows it. */
export interface PaletteEntry {
  entry: CatalogEntry
  /** The command's name in words, translated when a label exists. */
  label: string
}

/** The command's label: the palette's own, else the history's, else its name in words (`transport.set_tempo` is "transport set tempo"). */
export function labelOf(t: (key: MessageKey) => string, name: string): string {
  for (const prefix of [PALETTE_LABEL_PREFIX, COMMAND_LABEL_PREFIX]) {
    const key = prefix + name
    if (key in messages.en) return t(key as MessageKey)
  }
  return name.replace(/[._]/g, ' ')
}

/** The entries of the palette, in the catalogue's order. */
export function paletteEntries(t: (key: MessageKey) => string, catalog: readonly CatalogEntry[] = COMMAND_CATALOG): PaletteEntry[] {
  return catalog.map((entry) => ({ entry, label: labelOf(t, entry.name) }))
}

/** Lowercase and without accents, so that "mesure" finds "mesuré" and "e" finds "é". */
function fold(text: string): string {
  return text.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase()
}

/**
 * The entries a query finds, best first. Every word of the query (split on spaces, dots and underscores) must be in the
 * command's name, its label or its summary. A command ranks by where its words fall: in the name's own segments (the word
 * starts a segment of `transport.set_tempo`) first, then anywhere in the name or the label, then in the summary only; ties
 * keep the catalogue's order. An empty query lists them all in the catalogue's order.
 */
export function searchCommands(entries: readonly PaletteEntry[], query: string): PaletteEntry[] {
  const words = fold(query)
    .split(/[\s._]+/)
    .filter((w) => w !== '')
  if (words.length === 0) return [...entries]
  const scored: { item: PaletteEntry; score: number; index: number }[] = []
  entries.forEach((item, index) => {
    const name = fold(item.entry.name)
    const segments = name.split(/[._]/)
    const label = fold(item.label)
    const summary = fold(item.entry.summary)
    let score = 0
    for (const word of words) {
      if (segments.some((s) => s.startsWith(word))) score += 0
      else if (name.includes(word) || label.includes(word)) score += 1
      else if (summary.includes(word)) score += 3
      else return
    }
    scored.push({ item, score, index })
  })
  return scored.sort((a, b) => a.score - b.score || a.index - b.index).map((s) => s.item)
}

/** What the page knows that can fill a field in: the track the user selected. */
export interface Context {
  selectedTrack: string | null
}

/** The text a field starts with: the selected track for a track identifier, else nothing. */
export function initialText(field: CommandField, context: Context): string {
  if (field.ref === 'TrackId' && context.selectedTrack) return context.selectedTrack
  // A required switch starts off; an optional one starts on "default" (empty), so the command's own default holds.
  if (field.kind === 'boolean' && field.required) return 'false'
  return ''
}

/** What is wrong with a field: it needs an answer, or its answer cannot be read as what it takes. */
export type FieldProblem = 'required' | 'invalid'

export type Built = { ok: true; command: Command } | { ok: false; problems: Record<string, FieldProblem> }

/** One field's text as the value it stands for: `undefined` for "not given", `null` for `null`, or a problem. */
function parseField(field: CommandField, raw: string): { value: unknown } | { problem: FieldProblem } | 'omit' {
  const text = field.kind === 'string' ? raw : raw.trim()
  if (text === '') {
    // Never `null` for an empty field: for a level it means silence, so `null` must be typed (below).
    return field.required ? { problem: 'required' } : 'omit'
  }
  // A free text (a name) keeps the word "null"; an identifier, a number and the rest take it as `null`.
  if (field.nullable && text === 'null' && !(field.kind === 'string' && !field.ref)) return { value: null }
  switch (field.kind) {
    case 'boolean':
      return text === 'true' || text === 'false' ? { value: text === 'true' } : { problem: 'invalid' }
    case 'string':
      return { value: text }
    case 'integer': {
      const n = Number(text)
      return /^[+-]?\d+$/.test(text) && Number.isSafeInteger(n) ? { value: n } : { problem: 'invalid' }
    }
    case 'number': {
      const n = Number(text)
      return Number.isFinite(n) ? { value: n } : { problem: 'invalid' }
    }
    case 'enum':
      return field.options?.includes(text) ? { value: text } : { problem: 'invalid' }
    case 'json':
      try {
        return { value: JSON.parse(text) as unknown }
      } catch {
        return { problem: 'invalid' }
      }
  }
}

/**
 * The command an entry makes from the fields' texts. An optional field left empty is left out (the command's own default
 * holds); a required one left empty is `required`, a nullable one too: `null` is only what is typed (`null`), never what is left empty (for a level it is silence). The result is only
 * shaped: the project core still checks it and refuses what does not hold, as for any caller.
 */
export function buildCommand(entry: CatalogEntry, texts: Readonly<Record<string, string>>): Built {
  const problems: Record<string, FieldProblem> = {}
  const input: Record<string, unknown> = { type: entry.name }
  for (const field of entry.fields) {
    const parsed = parseField(field, texts[field.name] ?? '')
    if (parsed === 'omit') continue
    if ('problem' in parsed) problems[field.name] = parsed.problem
    else input[field.name] = parsed.value
  }
  if (Object.keys(problems).length > 0) return { ok: false, problems }
  return { ok: true, command: input as unknown as Command }
}
