/**
 * Computer keyboard as a piano. Keys are matched by physical position
 * (`KeyboardEvent.code`), so the layout is the same on QWERTY and AZERTY.
 *
 * The rule between playing and shortcuts (UI-3): no shortcut fires while the keyboard plays, meaning no
 * key does both: a played key never fires a shortcut, and a shortcut never plays a note. A shortcut
 * pressed while a note is held still works (undo stays reachable); the held note goes on.
 * - A played key is a bare letter or Semicolon (the keys of `KEY_OFFSETS`, plus Z and X for the octave).
 * - An application shortcut never uses a bare letter: it carries Ctrl or Cmd (`historyShortcut`), or is
 *   a key that plays nothing (F2 to rename, Escape, Enter, the arrows, Delete).
 * - So playing ignores any event with Ctrl, Cmd or Alt held (`ignoreKeyEvent`) and the shortcuts ignore
 *   bare letters; a new shortcut must keep to this, and `keyboard.test.ts` enumerates both sides.
 */

/** Semitone offset from C of each key, two rows like a piano. */
const KEY_OFFSETS: Record<string, number> = {
  KeyA: 0, KeyW: 1, KeyS: 2, KeyE: 3, KeyD: 4, KeyF: 5, KeyT: 6, KeyG: 7,
  KeyY: 8, KeyH: 9, KeyU: 10, KeyJ: 11, KeyK: 12, KeyO: 13, KeyL: 14, KeyP: 15, Semicolon: 16,
}

export const MIN_OCTAVE = 0
export const MAX_OCTAVE = 8
export const DEFAULT_OCTAVE = 4
/** The octave that puts the drum sounds (notes 36 to 46: kick A, snare S, clap E, closed hat T, open hat U) on the key row. */
export const DRUMS_OCTAVE = 2

/** The octave the keyboard starts at for a track whose instrument is of this kind (`faulk.drums` or any other, or none). */
export function defaultOctave(instrumentKind: string | null): number {
  return instrumentKind === 'faulk.drums' ? DRUMS_OCTAVE : DEFAULT_OCTAVE
}

/**
 * The application shortcuts that need no modifier, by `KeyboardEvent.key` (Space by `code`): F2 renames
 * a track, Escape cancels (a rename, the add-track menu, a tempo edit), Enter confirms a rename or a
 * tempo, the arrows step the tempo field, Space plays and stops the transport. With the time grid
 * focused, Left and Right scroll, Home goes to the start, plus and minus (and the keys that share
 * them) zoom. With the launch grid focused, the arrows move between cells, and a scene's name is renamed with F2 (Enter confirms, Escape cancels, as for a track). With the piano roll focused, Delete and Backspace remove the selected notes, Escape clears the
 * selection, and Ctrl or Cmd + A selects all. None of them plays a note: the playing keys are letters and Semicolon.
 */
export const BARE_SHORTCUT_CODES: readonly string[] = ['F2', 'Escape', 'Enter', 'ArrowUp', 'ArrowDown', 'Space', 'ArrowLeft', 'ArrowRight', 'Home', '+', '=', '-', '_', 'Delete', 'Backspace']

/** Every physical key that plays a note, for the rule above. */
export const PLAYING_KEYS: readonly string[] = Object.keys(KEY_OFFSETS)

/** MIDI note for a key at an octave (C4 = 60), or null when the key plays nothing. */
export function noteForKey(code: string, octave: number): number | null {
  const offset = KEY_OFFSETS[code]
  if (offset === undefined) return null
  const note = (octave + 1) * 12 + offset
  return note >= 0 && note <= 127 ? note : null
}

/** Octave change for Z (down) and X (up), by physical position. */
export function octaveStep(code: string): -1 | 1 | 0 {
  if (code === 'KeyZ') return -1
  if (code === 'KeyX') return 1
  return 0
}

export function clampOctave(octave: number): number {
  return Math.min(MAX_OCTAVE, Math.max(MIN_OCTAVE, octave))
}

/** Input types where the user types text: letters must not play notes there. */
const TEXT_INPUT_TYPES = new Set(['text', 'search', 'number', 'email', 'password', 'tel', 'url', 'date', 'time'])

/**
 * True when the event must not play: key repeat, modifier held, or typing text.
 * Knobs (range inputs), buttons, radios and checkboxes keep the keyboard playing,
 * so a sound can be tweaked with the mouse while notes are held.
 */
export function ignoreKeyEvent(event: Pick<KeyboardEvent, 'repeat' | 'ctrlKey' | 'metaKey' | 'altKey' | 'target'>): boolean {
  if (event.repeat || event.ctrlKey || event.metaKey || event.altKey) return true
  return isTextTarget(event.target)
}

/** True when the event's target is a field where the user types text (the browser's own shortcuts apply there). */
export function isTextTarget(eventTarget: EventTarget | null): boolean {
  const target = eventTarget as HTMLElement | null
  if (!target || typeof target.closest !== 'function') return false
  if (target.closest('textarea, select, [contenteditable=""], [contenteditable="true"]')) return true
  return target instanceof HTMLInputElement && TEXT_INPUT_TYPES.has(target.type)
}

/** Undo or redo asked by the keyboard (Ctrl or Cmd + Z, Shift for redo, Ctrl + Y), or null; never inside a text field. */
export function historyShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'target'>): 'undo' | 'redo' | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || isTextTarget(event.target)) return null
  const key = event.key.toLowerCase()
  if (key === 'z') return event.shiftKey ? 'redo' : 'undo'
  if (key === 'y' && !event.shiftKey) return 'redo'
  return null
}
