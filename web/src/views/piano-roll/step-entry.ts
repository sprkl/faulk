/**
 * Step entry in the piano roll (#882): a mode in which the musical-typing letters write notes at the free cursor
 * (#494) instead of playing live. The pure part: which key means what, and the bookkeeping of the steps written
 * (so that Backspace and the lengthen key know what the last step was). The view sends the commands.
 *
 * Keys, all by position (`KeyboardEvent.code`) so the layout does not matter, none a note, none a shortcut already:
 * - the letters of musical typing write a note at the cursor, of the toolbar's length; the cursor advances by that
 *   length when the last letter held is released, so letters held together are a chord on one step;
 * - Period (".") is a rest: the cursor advances, nothing is written;
 * - Comma (",") lengthens the last step by the toolbar's note length, whatever the snap (a rest included) and puts the cursor at its end;
 * - Backspace removes the last step's note(s) and puts the cursor back where the step began.
 */
import type { Note } from '../../lib/generated/protocol'
import { noteForKey } from '../../lib/keyboard'
import { stepLengths, type Change } from './roll'

/** The rest key: Period by position. It is no playing key (`KEY_OFFSETS` has none) and no bare shortcut. */
export const REST_CODE = 'Period'
/** The lengthen key: Comma by position. */
export const LENGTHEN_CODE = 'Comma'
/** The key that switches the mode on and off: Alt+E, by position (never bare: E plays a note). */
export const STEP_ENTRY_CODE = 'KeyE'

export type StepKey = { kind: 'note'; code: string } | { kind: 'rest' } | { kind: 'lengthen' } | { kind: 'back' }

type KeyFacts = Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'repeat'>

/** What a key event means while the mode is on, or null: Ctrl, Cmd or Alt make a key a shortcut, and a repeat is no new step. */
export function stepEntryKey(event: KeyFacts): StepKey | null {
  if (event.ctrlKey || event.metaKey || event.altKey || event.repeat) return null
  if (event.code === 'Backspace') return event.shiftKey ? null : { kind: 'back' }
  if (event.code === REST_CODE) return event.shiftKey ? null : { kind: 'rest' }
  if (event.code === LENGTHEN_CODE) return event.shiftKey ? null : { kind: 'lengthen' }
  return noteForKey(event.code, 4) === null ? null : { kind: 'note', code: event.code }
}

/** Alt+E: the mode's switch. No repeat; the caller leaves text fields out. */
export function isStepEntryToggle(event: KeyFacts): boolean {
  return event.code === STEP_ENTRY_CODE && event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && !event.repeat
}

/** One step written: where it began, how far it goes, and the notes it holds (none for a rest). */
export interface EntryStep {
  start: number
  length: number
  notes: readonly Note[]
}

/** Where the cursor goes after a step that ends at `end`, or null when that is the clip's end (no room for another step). */
export function advanceTo(end: number, clipLength: number): number | null {
  return end < clipLength ? end : null
}

/** The lengthen key's result: the last step grown by one unit (the note length the toolbar chose, so the cursor stays on that grid). */
export interface Lengthened {
  /** The notes to update (none for a rest, or when the clip's end leaves no room). */
  changes: Change[]
  steps: EntryStep[]
  /** Where the cursor goes: the end of the lengthened step. */
  cursor: number
  /** The step could not grow by a whole unit: the clip ends first. */
  cut: boolean
}

/** Null when no step was written. */
export function lengthenLast(steps: readonly EntryStep[], unit: number, clipLength: number): Lengthened | null {
  const last = steps.at(-1)
  if (!last) return null
  const head = steps.slice(0, -1)
  if (last.notes.length === 0) {
    // A rest grows too: more silence.
    const length = Math.min(last.length + unit, Math.max(last.length, clipLength - last.start))
    return { changes: [], steps: [...head, { ...last, length }], cursor: last.start + length, cut: length < last.length + unit }
  }
  const { changes, cut } = stepLengths(last.notes, unit, 1, clipLength)
  const grown = changes.filter((c) => c.to.length > c.from.length)
  const notes = last.notes.map((n) => changes.find((c) => c.from === n)?.to ?? n)
  const length = Math.max(last.length, ...notes.map((n) => n.length))
  return { changes: grown, steps: [...head, { ...last, notes, length }], cursor: last.start + length, cut }
}

/** Backspace: the last step goes, its notes with it (the caller has refreshed them to what the clip holds now), and the cursor goes back to where it began. */
export function dropLast(steps: readonly EntryStep[]): { steps: EntryStep[]; notes: Note[]; cursor: number } | null {
  const last = steps.at(-1)
  if (!last) return null
  return { steps: steps.slice(0, -1), notes: [...last.notes], cursor: last.start }
}

/** Bar and beat (both from 1, the beat to a hundredth) of a tick, for the words said. */
export function barBeat(tick: number, ppq: number, beatsPerBar: number): { bar: number; beat: number } {
  const beat = tick / ppq
  return { bar: Math.floor(beat / beatsPerBar) + 1, beat: Number(((beat % beatsPerBar) + 1).toFixed(2)) }
}
