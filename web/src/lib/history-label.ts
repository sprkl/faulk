/**
 * The words of a history row (#473): what was done, to which objects, by name. "Mute Lead", "Delete 17 clips (Bass,
 * Drums, Lead)", "Add 4 notes (Bass 1)". The project core gathers the names ({@link Subject}); this builds the sentence
 * in the user's language from a phrase per command (`history.say.<name>`, `.on` and `.off` for a switch) and a noun.
 * A command without a phrase, or whose entry says nothing of its objects, keeps its generic label (`history.cmd.<name>`).
 */
import { DEVICES, type ParamRef, type Subject } from './generated/protocol'
import { formatValue, isChoice, labelKey, snap } from './devices'
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

/**
 * The names of a list: the ones the objects have, then the default name of each that has none ("Track 2", "Scene 3": the
 * name the rest of the page gives it from its position). Kept as one list so that the counts and the first few agree.
 */
function names(named: readonly string[], unnamed: readonly number[], key: 'tracks.defaultName' | 'grid.sceneDefault', t: T): string[] {
  return [...named, ...unnamed.map((n) => t(key, { n }))]
}

/** "Lead", "Bass 1 (Bass)", "17 clips (Bass, Drums, Lead)", "a note (Bass 1)", "Track 2 (Oscillator)". */
export function what(subject: Subject, noun: Noun, t: T): string {
  const objects = names(subject.objects, subject.unnamed, noun === 'scene' ? 'grid.sceneDefault' : 'tracks.defaultName', t)
  const within = names(subject.within, subject.within_unnamed, 'tracks.defaultName', t).filter((name) => !objects.includes(name))
  const around = within.length > 0 ? ` (${list(within)})` : ''
  if (subject.count <= 1 && objects.length === 1) return `${objects[0]}${around}`
  if (subject.count <= 1 && objects.length === 0) return `${t(`history.noun.${noun}.one` as MessageKey)}${around}`
  const count = Math.max(subject.count, objects.length)
  return `${t(`history.noun.${noun}.many` as MessageKey, { count })}${around}`
}

/** A parameter's translated label ("Cutoff", "Volume"), without its device; its id when the page has no label for it. */
export function paramLabel(ref: ParamRef, t: T): string {
  // Only the mixer's parameters have no device; a module has a device but no kind to look a label up with.
  if (ref.device == null) return t(ref.key === 'gain' ? 'automation.volume' : 'automation.pan')
  const d = DEVICES.find((device) => device.kind === ref.kind)
  const key = d ? labelKey(d, ref.key) : ''
  return has(key) ? t(key) : ref.key
}

/** "Filter › Cutoff", "Volume": what a lane drives, as the lane's own header says it. */
export function paramPath(ref: ParamRef, t: T): string {
  const label = paramLabel(ref, t)
  return ref.device ? `${ref.device} › ${label}` : label
}

/** A value of a parameter as its device panel shows it ("1.2 kHz", "Saw"); the number itself for a parameter the page does not know. */
export function valueLabel(ref: ParamRef, value: number, t: T): string {
  const d = DEVICES.find((device) => device.kind === ref.kind)
  const param = d?.params.find((p) => p.key === ref.key)
  if (!d || !param) return `${+value.toFixed(3)}`
  if (isChoice(param)) {
    const chosen = param.values[Math.round((snap(param, value) - param.min) / (param.step || 1))]
    const key = chosen === undefined ? '' : labelKey(d, chosen)
    return has(key) ? t(key) : formatValue(param, snap(param, value))
  }
  return formatValue(param, snap(param, value))
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
  // A parameter named with its value, or alone when the entry holds several values (a drag, a group).
  const valued = subject.param ? `${PHRASE_PREFIX}${name}.${subject.value != null ? 'value' : 'param'}` : null
  const key = switched && has(switched) ? switched : valued && has(valued) ? valued : `${PHRASE_PREFIX}${name}`
  if (!has(key)) return null
  const text = messages.en[key]
  if (text.includes('{to}') && subject.to == null) return null
  // A lane's phrase names the parameter it drives and the track it is on; the object is then only a count of points.
  const lane = text.includes('{lane}')
  if (lane && !subject.param) return null
  const needsWhat = text.includes('{what}')
  const unnamed = subject.unnamed.length + subject.within_unnamed.length
  if (needsWhat && subject.count === 0 && subject.objects.length === 0 && subject.within.length === 0 && unnamed === 0) return null
  const objects = lane ? { ...subject, within: [], within_unnamed: [] } : subject
  const around = lane ? names(subject.within, subject.within_unnamed, 'tracks.defaultName', t) : []
  const param = subject.param
  return t(key, {
    what: needsWhat ? what(objects, nounOf(name), t) : '',
    to: subject.to ?? '',
    lane: lane && param ? [paramPath(param, t), ...(around.length > 0 ? [list(around)] : [])].join(', ') : '',
    param: param ? paramLabel(param, t) : '',
    value: param && subject.value != null ? valueLabel(param, subject.value, t) : '',
  })
}
