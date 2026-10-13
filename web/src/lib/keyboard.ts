/**
 * Computer keyboard as a piano. Keys are matched by physical position
 * (`KeyboardEvent.code`), so the layout is the same on QWERTY and AZERTY.
 */

/** Semitone offset from C of each key, two rows like a piano. */
const KEY_OFFSETS: Record<string, number> = {
  KeyA: 0, KeyW: 1, KeyS: 2, KeyE: 3, KeyD: 4, KeyF: 5, KeyT: 6, KeyG: 7,
  KeyY: 8, KeyH: 9, KeyU: 10, KeyJ: 11, KeyK: 12, KeyO: 13, KeyL: 14, KeyP: 15, Semicolon: 16,
}

export const MIN_OCTAVE = 0
export const MAX_OCTAVE = 8
export const DEFAULT_OCTAVE = 4

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
  const target = event.target as HTMLElement | null
  if (!target || typeof target.closest !== 'function') return false
  if (target.closest('textarea, select, [contenteditable=""], [contenteditable="true"]')) return true
  return target instanceof HTMLInputElement && TEXT_INPUT_TYPES.has(target.type)
}
