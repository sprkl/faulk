/**
 * What the command palette asks a musician (#926), field by field: the identifiers and lists the page can fill from what is
 * selected are filled and not shown, a command that needs a selection the page does not have says so instead of asking for an
 * identifier or JSON, positions and lengths are typed in bars and beats like "Go to", every shown field has a label in the
 * language of the interface with its unit, and a field that has a current value starts on it. Pure: the page gives what it
 * knows (`PaletteContext`), the palette shows the result and `command-palette.ts` builds the command from the texts.
 */
import type { MessageKey } from './i18n'
import type { CatalogEntry, CommandField, Note } from './generated/protocol'
import { barTicks, beatTicks, parseBarBeat, PPQ, type Signature } from '../views/transport/time'

/** What the page knows that can fill the fields in. */
export interface PaletteContext {
  selectedTrack: string | null
  /** The clip open in the piano roll. */
  clip: string | null
  /** The scenes, the markers and the automation lanes of the selected track the user can choose from. */
  scenes: readonly { id: string; name: string }[]
  markers: readonly { id: string; name: string }[]
  lanes: readonly { id: string; name: string }[]
  /** The placements selected in the arrangement. */
  placements: readonly { id: string; clip: string }[]
  /** The notes selected in the piano roll, of the open clip. */
  notes: readonly Note[]
  /** The devices of the selected track, in chain order, with the name to show. */
  devices: readonly { id: string; name: string; kind?: string }[]
  signature: Signature
  /** Current values: `<command>.<field>` to the text the field starts on. */
  current: Readonly<Record<string, string>>
}

/** What a field stands for: what the page fills in, or what the user is asked. */
export type FieldSource = 'ask' | 'track' | 'clip' | 'placement' | 'placements' | 'notes' | 'device' | 'none'

/** The unit of a tick field: a position is typed as "Go to" (`33`, `33.2`, `33.2.3`), a length in bars, a step in beats. */
export type TickUnit = 'position' | 'bars' | 'beats'

const POSITIONS = new Set(['at', 'from', 'start', 'tick', 'to', 'until'])
const BARS = new Set(['length', 'by', 'offset'])
const BEATS = new Set(['grid'])

/** The unit of a field of kind tick (`ref` `Tick`), or `null` for any other field and for a tick field no unit is decided for (a test fails on it). */
export function tickUnit(field: CommandField): TickUnit | null {
  if (field.ref !== 'Tick') return null
  if (POSITIONS.has(field.name)) return 'position'
  if (BARS.has(field.name)) return 'bars'
  return BEATS.has(field.name) ? 'beats' : null
}

/** The ref kinds of identifiers the page cannot fill: the command says what to select instead. */
export type Need = 'track' | 'clip' | 'placement' | 'placements' | 'notes' | 'device' | 'scene' | 'lane' | 'marker' | 'preset'

const NEEDS: Readonly<Record<string, Need>> = {
  TrackId: 'track',
  ClipId: 'clip',
  PlacementId: 'placement',
  DeviceId: 'device',
  SceneId: 'scene',
  LaneId: 'lane',
  MarkerId: 'marker',
  PresetRef: 'preset',
}

/** The key of the sentence that says what a filled field stands for: one thing and several things are not said alike. */
export function usesKey(source: Exclude<FieldSource, 'none'>, count: number): MessageKey {
  return (count === 1 && (source === 'notes' || source === 'placements') ? `palette.uses.${source}.one` : `palette.uses.${source}`) as MessageKey
}

/** One field as the form shows it. */
export interface ResolvedField {
  field: CommandField
  source: FieldSource
  /** What the field starts on (the text the command is built from, also for a field that is not shown). */
  text: string
  /** The field is asked of the user. */
  shown: boolean
  /** The page needs this from a selection it does not have: the command cannot run. */
  missing?: Need
  /** A choice among the selected track's devices, its lanes, the scenes or the markers. */
  choices?: readonly { value: string; label: string }[]
  /** How many things the filled field stands for (notes, placements), for the sentence that says it. */
  count?: number
}

/** One field resolved against what the page knows. */
export function resolveField(entry: CatalogEntry, field: CommandField, ctx: PaletteContext): ResolvedField {
  const ref = field.ref ?? ''
  const base = { field, shown: false }
  const need = (n: Need): ResolvedField => (field.required ? { ...base, source: 'none', text: '', missing: n } : { ...base, source: 'none', text: '' })
  if (ref === 'TrackId' || ref === 'Id') {
    // An optional track (the clip's destination) is never guessed from the selection; a device's is.
    if (!field.required && !entry.name.startsWith('device.')) return { ...base, source: 'none', text: '' }
    return ctx.selectedTrack ? { ...base, source: 'track', text: ctx.selectedTrack } : need('track')
  }
  if (ref === 'ClipId') {
    // Notes edit the clip open in the roll; the other commands the clip of the one placement selected in the timeline, else the open one.
    const placed = ctx.placements.length === 1 ? ctx.placements[0]!.clip : null
    const clip = entry.fields.some((f) => f.name === 'notes') ? (ctx.clip ?? placed) : (placed ?? ctx.clip)
    return clip ? { ...base, source: 'clip', text: clip } : need('clip')
  }
  if (ref === 'PlacementId') {
    const only = ctx.placements.length === 1 ? ctx.placements[0] : undefined
    return only ? { ...base, source: 'placement', text: only.id } : need('placement')
  }
  if (field.name === 'placements' && field.kind === 'json') {
    if (ctx.placements.length === 0) return need('placements')
    return { ...base, source: 'placements', text: JSON.stringify(ctx.placements.map((p) => p.id)), count: ctx.placements.length }
  }
  if (field.name === 'notes' && field.kind === 'json') {
    if (ctx.notes.length === 0) return need('notes')
    return { ...base, source: 'notes', text: JSON.stringify(ctx.notes), count: ctx.notes.length }
  }
  if (ref === 'DeviceId') {
    if (ctx.devices.length === 0) return need('device')
    const first = ctx.devices[0]!
    if (ctx.devices.length === 1) return { ...base, source: 'device', text: first.id }
    return { ...base, source: 'device', text: first.id, shown: true, choices: ctx.devices.map((d) => ({ value: d.id, label: d.name })) }
  }
  // A scene, a marker or a lane is chosen, never guessed: an empty choice is a field the user fills.
  const list = ref === 'SceneId' ? ctx.scenes : ref === 'MarkerId' ? ctx.markers : ref === 'LaneId' ? ctx.lanes : null
  if (list) {
    if (list.length === 0) return need(NEEDS[ref]!)
    return { ...base, source: 'none', text: '', shown: true, choices: list.map((x) => ({ value: x.id, label: x.name })) }
  }
  const unresolvable = NEEDS[ref]
  if (unresolvable) return need(unresolvable)
  // Where the device goes is the selected track's: the master's chain is not asked for here, so no field offers both.
  if (field.name === 'master' && entry.name === 'device.insert') return { ...base, source: 'none', text: '' }
  // Structured input and internal values nobody types: never shown (a required one makes the command hidden, see `palette-hidden.ts`).
  if (field.kind === 'json') return { ...base, source: 'none', text: '' }
  const current = ctx.current[`${entry.name}.${field.name}`]
  const initial = current ?? (field.kind === 'boolean' && field.required ? 'false' : '')
  return { field, source: 'ask', text: initial, shown: true }
}

/** Every field of a command resolved, and the first thing a missing selection asks for (null: the command can run). */
export function resolveFields(entry: CatalogEntry, ctx: PaletteContext): { fields: ResolvedField[]; missing: Need | null } {
  const fields = entry.fields.map((f) => resolveField(entry, f, ctx))
  return { fields, missing: fields.find((f) => f.missing)?.missing ?? null }
}

/**
 * The ticks a text means for a field of a unit, or `null` when it means nothing. A position is "Go to"'s (`33`, `33.2`,
 * `33.2.3`); a length is a number of bars (`2`, `0.5`; a comma works too; a distance may be negative); a step is a number of beats.
 */
export function parseTickText(unit: TickUnit, text: string, sig: Signature, signed: boolean): number | null {
  const t = text.trim()
  if (unit === 'position') return parseBarBeat(t, sig)
  const found = /^([+-]?)(\d+(?:[.,]\d+)?)$/.exec(t)
  if (!found || (found[1] !== '' && !signed)) return null
  const amount = Number(found[2]!.replace(',', '.'))
  const ticks = Math.round(amount * (unit === 'bars' ? barTicks(sig) : beatTicks(sig)))
  if (!Number.isFinite(ticks) || (unit === 'bars' && ticks === 0 && amount !== 0)) return null
  return found[1] === '-' ? -ticks : ticks
}

/** A tick position as "Go to" would take it back: bar.beat (the sixteenth only when it is not on the beat). */
export function positionText(tick: number, sig: Signature): string {
  const whole = Math.max(0, Math.round(tick))
  const bar = Math.floor(whole / barTicks(sig))
  const inBar = whole - bar * barTicks(sig)
  const beat = Math.floor(inBar / beatTicks(sig))
  const sixteenth = Math.floor((inBar - beat * beatTicks(sig)) / (PPQ / 4))
  return sixteenth === 0 ? `${bar + 1}.${beat + 1}` : `${bar + 1}.${beat + 1}.${sixteenth + 1}`
}
