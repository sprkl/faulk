/**
 * The command palette (#108): the pure part. It lists the commands of the catalogue (`COMMAND_CATALOG`, generated from the
 * Rust catalogue: every `edit`, `live` and `setting` command, never a hand-written list), finds them by what is typed, and
 * turns what is typed into a field into a `Command` the dispatcher runs, like every other action. It owns no path of its own.
 */
import { COMMAND_CATALOG, type CatalogEntry, type Command, type CommandField } from './generated/protocol'
import type { MessageKey } from './i18n'
import { messages } from './i18n/app'
import { COMMAND_LABEL_PREFIX } from './i18n/history'
import { GESTURES, type GestureId } from './palette-gestures'
import { parseTickText, tickUnit, type TickUnit } from './palette-fields'
import type { Signature } from '../views/transport/time'
import { HIDDEN_COMMANDS } from './palette-hidden'
import { END_OF_LIST, isListIndex, NONE } from './palette-choices'

/** The key of a command's own palette label (`palette.cmd.<name>`), for the live commands the history never shows. */
export const PALETTE_LABEL_PREFIX = 'palette.cmd.'

/** The key of a command's one-line summary (`palette.sum.<name>`). */
export const SUMMARY_PREFIX = 'palette.sum.'

/** The most results the palette shows at once: the list stays short enough to read, and the rest is found by typing more. */
export const MAX_RESULTS = 50

/** What a palette entry stands for: a command of the catalogue, or a gesture of the interface (`palette-gestures.ts`). */
export interface PaletteEntry {
  entry: CatalogEntry
  /** The title: translated, never the name of the command in words for a command listed by default. */
  label: string
  /** One line in the language of the interface; for a command listed only after `>`, the catalogue's own (English) summary. */
  summary: string
  /** Not listed unless the search starts with `>` (`palette-hidden.ts`). */
  hidden: boolean
  /** Set for a gesture of the interface: it runs through the view that owns it, and `entry` only carries its key `ui.<id>`. */
  gesture?: GestureId
  /** The keys that do the same, as the registry names them (a label of `SHORTCUT_GROUPS`). */
  shortcut?: MessageKey
}

/** The command's label: the palette's own, else the history's, else its name in words (`transport.set_tempo` is "transport set tempo"). */
export function labelOf(t: (key: MessageKey) => string, name: string): string {
  for (const prefix of [PALETTE_LABEL_PREFIX, COMMAND_LABEL_PREFIX]) {
    const key = prefix + name
    if (key in messages.en) return t(key as MessageKey)
  }
  return name.replace(/[._]/g, ' ')
}

/** The label of a field in the language of the interface, with its unit: the command's own (`palette.field.<command>.<field>`), else the field's. */
export function fieldLabel(t: (key: MessageKey) => string, command: string, field: CommandField): string {
  for (const key of [`palette.field.${command}.${field.name}`, `palette.field.${field.name}`]) if (key in messages.en) return t(key as MessageKey)
  return field.name.replace(/_/g, ' ')
}

/** The one-line summary the palette shows for a command: the page's own, else the catalogue's. */
export function summaryOfCommand(t: (key: MessageKey) => string, entry: CatalogEntry): string {
  const key = SUMMARY_PREFIX + entry.name
  return key in messages.en ? t(key as MessageKey) : entry.summary
}

/** The entries of the palette: the catalogue's commands in its order, then the gestures of the interface. Hidden ones are marked, not left out. */
export function paletteEntries(t: (key: MessageKey) => string, catalog: readonly CatalogEntry[] = COMMAND_CATALOG): PaletteEntry[] {
  const commands = catalog.map((entry) => ({
    entry,
    label: labelOf(t, entry.name),
    summary: summaryOfCommand(t, entry),
    hidden: HIDDEN_COMMANDS.has(entry.name),
  }))
  const gestures = GESTURES.map((g): PaletteEntry => ({
    entry: { name: `ui.${g.id}`, kind: 'live', summary: '', fields: [] },
    label: t(`palette.gesture.${g.id}` as MessageKey),
    summary: t(`palette.gsum.${g.id}` as MessageKey),
    hidden: false,
    gesture: g.id,
    shortcut: g.shortcut,
  }))
  return [...commands, ...gestures]
}

/** What the search field holds, read: a `>` first asks for the whole catalogue, the hidden commands and the agents' names included. */
export function parseQuery(query: string): { all: boolean; text: string } {
  const trimmed = query.trimStart()
  return trimmed.startsWith('>') ? { all: true, text: trimmed.slice(1) } : { all: false, text: query }
}

/** Lowercase and without accents, so that "mesure" finds "mesuré" and "e" finds "é". */
function fold(text: string): string {
  return text.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase()
}

/**
 * The entries a query finds, best first. Every word of the query (split on spaces, dots and underscores) must be in the
 * command's name, its label, its keys or its summary. A command ranks by where its words fall: in the name's own segments
 * (the word starts a segment of `transport.set_tempo`) or in its label first, then anywhere in the name, then in the summary or the keys only; ties
 * keep the catalogue's order. An empty query lists them all in the catalogue's order. `keys` gives the keys an entry shows
 * ("Ctrl+Shift+L"), so that a key is found too.
 */
export function searchCommands(entries: readonly PaletteEntry[], query: string, keys: (item: PaletteEntry) => string = () => ''): PaletteEntry[] {
  const words = fold(query)
    .split(/[\s._]+/)
    .filter((w) => w !== '')
  if (words.length === 0) return [...entries]
  const scored: { item: PaletteEntry; score: number; index: number }[] = []
  entries.forEach((item, index) => {
    // A gesture has no agent name.
    const name = item.gesture ? '' : fold(item.entry.name)
    const segments = name.split(/[._]/)
    const label = fold(item.label)
    const labelWords = label.split(/[^a-z0-9]+/)
    const summary = fold(item.summary)
    const shortcut = fold(keys(item))
    let score = 0
    for (const word of words) {
      if (segments.some((s) => s !== '' && s.startsWith(word)) || labelWords.some((w) => w.startsWith(word))) score += 0
      else if ((name !== '' && name.includes(word)) || label.includes(word)) score += 1
      else if (summary.includes(word) || shortcut.includes(word)) score += 3
      else return
    }
    scored.push({ item, score, index })
  })
  return scored.sort((a, b) => a.score - b.score || a.index - b.index).map((s) => s.item)
}

/** What the palette lists for what is typed: the default level (no hidden command) or, after `>`, the whole catalogue. */
export function listFor(entries: readonly PaletteEntry[], query: string, keys?: (item: PaletteEntry) => string): PaletteEntry[] {
  const { all, text } = parseQuery(query)
  return searchCommands(all ? entries : entries.filter((e) => !e.hidden), text, keys)
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
  // The "none" of a list (a track without a colour) is `null`, whatever the field's kind.
  if (field.nullable && text === NONE) return { value: null }
  // A place in a list is counted from 1 by the musician and from 0 by the command; empty is the end.
  if (isListIndex(field)) {
    if (text === '') return { value: END_OF_LIST }
    return /^\d+$/.test(text) && Number(text) >= 1 && Number.isSafeInteger(Number(text)) ? { value: Number(text) - 1 } : { problem: 'invalid' }
  }
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

/** A tick field's text as the ticks it stands for (`by` may be negative), or a problem. */
function tickField(field: CommandField, unit: TickUnit, raw: string, signature: Signature): { value: unknown } | { problem: FieldProblem } {
  if (field.nullable && raw.trim() === 'null') return { value: null }
  const ticks = parseTickText(unit, raw, signature, field.name === 'by')
  return ticks === null ? { problem: 'invalid' } : { value: ticks }
}

/**
 * The command an entry makes from the fields' texts. An optional field left empty is left out (the command's own default
 * holds); a required one left empty is `required`, a nullable one too: `null` is only what is typed (`null`), never what is left empty (for a level it is silence). The result is only
 * shaped: the project core still checks it and refuses what does not hold, as for any caller.
 */
export function buildCommand(entry: CatalogEntry, texts: Readonly<Record<string, string>>, signature: Signature = { numerator: 4, denominator: 4 }): Built {
  const problems: Record<string, FieldProblem> = {}
  const input: Record<string, unknown> = { type: entry.name }
  for (const field of entry.fields) {
    const unit = tickUnit(field)
    const raw = texts[field.name] ?? ''
    // A position or a length is typed in bars and beats (#926), then goes to the command in ticks.
    const parsed = unit && raw.trim() !== '' ? tickField(field, unit, raw, signature) : parseField(field, raw)
    if (parsed === 'omit') continue
    if ('problem' in parsed) problems[field.name] = parsed.problem
    else input[field.name] = parsed.value
  }
  if (Object.keys(problems).length > 0) return { ok: false, problems }
  return { ok: true, command: input as unknown as Command }
}
