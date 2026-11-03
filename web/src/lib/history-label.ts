/**
 * The words of a history row (#473): what was done, to which objects, by name. "Mute Lead", "Delete 17 clips (Bass,
 * Drums, Lead)", "Add 4 notes (Bass 1)". The project core gathers the names ({@link Subject}); this builds the sentence
 * in the user's language from a phrase per command (`history.say.<name>`, `.on` and `.off` for a switch) and a noun.
 * A command without a phrase, or whose entry says nothing of its objects, keeps its generic label (`history.cmd.<name>`).
 */
import type { Subject } from './generated/protocol'
import { COMMAND_LABEL_PREFIX, PHRASE_PREFIX } from './i18n/history'
import { messages, type MessageKey } from './i18n/app'

type T = (key: MessageKey, vars?: Record<string, string | number>) => string

/** The kind of object a command acts on, for the counted form ("17 clips"). */
export type Noun = 'clip' | 'note' | 'track' | 'device' | 'scene' | 'lane' | 'point'

/** The noun of a command's objects, from its name. */
export function nounOf(name: string): Noun {
  if (name.startsWith('clip.notes.') || name === 'record.commit_take') return 'note'
  if (name.startsWith('automation.points.')) return 'point'
  if (name.startsWith('automation.')) return 'lane'
  if (name === 'arrangement.place_scene' || name.startsWith('scene.') || name === 'launch.scene') return 'scene'
  if (name.startsWith('clip.') || name.startsWith('arrangement.') || name === 'launch.clip') return 'clip'
  if (name.startsWith('device.')) return 'device'
  return 'track'
}

/** How many names a list shows before "+n". */
const SHOWN = 3

function list(names: readonly string[]): string {
  const head = names.slice(0, SHOWN).join(', ')
  return names.length > SHOWN ? `${head} +${names.length - SHOWN}` : head
}

function has(key: string): key is MessageKey {
  return key in messages.en
}

/** "Lead", "Bass 1 (Bass)", "17 clips (Bass, Drums, Lead)", "a note (Bass 1)". */
export function what(subject: Subject, noun: Noun, t: T): string {
  const within = subject.within.filter((name) => !subject.objects.includes(name))
  const around = within.length > 0 ? ` (${list(within)})` : ''
  if (subject.count <= 1 && subject.objects.length === 1) return `${subject.objects[0]}${around}`
  if (subject.count <= 1 && subject.objects.length === 0) return `${t(`history.noun.${noun}.one` as MessageKey)}${around}`
  const count = Math.max(subject.count, subject.objects.length)
  return `${t(`history.noun.${noun}.many` as MessageKey, { count })}${around}`
}

/** The generic label of a command: its translated `history.cmd.<name>`, else its name in words. */
export function generic(name: string, t: T): string {
  const key = COMMAND_LABEL_PREFIX + name
  return has(key) ? t(key) : name.replace(/[._]/g, ' ')
}

/** The label of a history row: the phrase naming its objects when there is one, else the generic label. */
export function label(row: { name: string; subject?: Subject | null; others?: number }, t: T): string {
  const base = phrase(row.name, row.subject, t) ?? generic(row.name, t)
  const others = row.others ?? 0
  return others > 0 ? t('history.others', { text: base, count: others }) : base
}

function phrase(name: string, subject: Subject | null | undefined, t: T): string | null {
  if (!subject) return null
  const switched = subject.on == null ? null : `${PHRASE_PREFIX}${name}.${subject.on ? 'on' : 'off'}`
  const key = switched && has(switched) ? switched : `${PHRASE_PREFIX}${name}`
  if (!has(key)) return null
  const text = messages.en[key]
  if (text.includes('{to}') && subject.to == null) return null
  const needsWhat = text.includes('{what}')
  if (needsWhat && subject.count === 0 && subject.objects.length === 0 && subject.within.length === 0) return null
  return t(key, { what: needsWhat ? what(subject, nounOf(name), t) : '', to: subject.to ?? '' })
}
