/**
 * Computer keyboard as a piano. Keys are matched by physical position
 * (`KeyboardEvent.code`), so the layout is the same on QWERTY and AZERTY.
 *
 * The computer keyboard plays in every view (#361); the order in which a key is shared is `playsNote`.
 *
 * The rule between playing and shortcuts (UI-3): no shortcut fires while the keyboard plays, meaning no
 * key does both: a played key never fires a shortcut, and a shortcut never plays a note. A shortcut
 * pressed while a note is held still works (undo stays reachable); the held note goes on.
 * - A played key is a bare letter or Semicolon (the keys of `KEY_OFFSETS`, plus Z and X for the octave).
 * - An application shortcut never uses a bare letter: it carries Ctrl or Cmd (`historyShortcut`, `saveShortcut`), or is
 *   a key that plays nothing (F2 to rename, Escape, Enter, the arrows, Delete).
 * - So playing ignores any event with Ctrl, Cmd or Alt held (`ignoreKeyEvent`) and the shortcuts ignore
 *   bare letters; a new shortcut must keep to this, and `keyboard.test.ts` enumerates both sides.
 */

/** Semitone offset from C of each key, two rows like a piano. */
const KEY_OFFSETS: Record<string, number> = {
  KeyA: 0,
  KeyW: 1,
  KeyS: 2,
  KeyE: 3,
  KeyD: 4,
  KeyF: 5,
  KeyT: 6,
  KeyG: 7,
  KeyY: 8,
  KeyH: 9,
  KeyU: 10,
  KeyJ: 11,
  KeyK: 12,
  KeyO: 13,
  KeyL: 14,
  KeyP: 15,
  Semicolon: 16,
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
 * The key that toggles musical typing (#470): the key left of 1 (` on QWERTY, ² on AZERTY), by position.
 * It plays nothing and no browser or page shortcut uses it bare, so it needs no modifier and is not a letter.
 */
export const TYPING_TOGGLE_CODE = 'Backquote'

/**
 * The application shortcuts that need no modifier, by `KeyboardEvent.key` (Space by `code`): F2 renames
 * a track (in its inspector), Escape cancels (a rename, the add-track menu, a tempo edit), Enter confirms a rename or a
 * tempo, the arrows step the tempo field, Space plays and stops the transport. With the time grid
 * focused, Left and Right scroll, Home goes to the start, plus and minus (and the keys that share
 * them) zoom, bare or with Ctrl or Cmd (#471: Ctrl keeps the browser's page zoom off while the grid has the focus). On the selected track's row of the arrangement (#471), Enter or F2 open the field that types its volume, and a digit, a sign or a decimal point opens it with that key typed (`views/arrangement/lane-keys.ts`); the row's height handle moves with Up and Down (Shift for a big step), Home and End, and Enter gives back the default height. Digits play nothing. With the launch grid focused, the arrows move between cells, and a scene's name is renamed with F2 (Enter confirms, Escape cancels, as for a track); on a clip, F2 renames it, Delete and Backspace remove it, and Ctrl or Cmd + C then + V copies it into an empty cell. With the piano roll focused, Delete and Backspace remove the selected notes, Escape clears the
 * selection, Ctrl or Cmd + A selects all, Ctrl or Cmd + C, X and V copy, cut and paste notes (pasted at the play position of the clip), Ctrl or Cmd + D duplicates the selection right after itself, and Up and Down transpose it by a semitone (an octave with Shift). Quantizing has no key: a bare letter plays a note and Ctrl + Q belongs to the browser. With a continuous control (`Slider`) focused, the arrows (with Ctrl, Cmd on macOS, for a fine step), Page Up, Page Down, Home and End move it and Delete and Backspace put its default value back (`continuous.ts`), Enter opens a field to type its value (Enter applies, Escape cancels; the field takes every key without Ctrl or Cmd); Tab closes its menu. With the bottom panel's handle focused (a separator), Up and Down resize the panel (Shift for a big step), Home and End go to its smallest and largest size, and Enter folds or unfolds it. The key left of 1 (Backquote, `TYPING_TOGGLE_CODE`) toggles musical typing (#470): with it off, the playing keys play nothing; it is not listed here, because it must not make a mouse-focused slider keyboard-focused. None of them plays a note: the playing keys are letters and Semicolon.
 */
export const BARE_SHORTCUT_CODES: readonly string[] = [
  'F2',
  'Escape',
  'Enter',
  'ArrowUp',
  'ArrowDown',
  'Space',
  'ArrowLeft',
  'ArrowRight',
  'Home',
  '+',
  '=',
  '-',
  '_',
  'Delete',
  'Backspace',
  'PageUp',
  'PageDown',
  'End',
  'Tab',
]

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
 * Knobs and faders (`role="slider"`), range inputs, buttons, radios and checkboxes keep the keyboard playing,
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

/** The elements that have a Space meaning of their own: Space presses a button or a link, ticks a box, opens a select, types in a field. */
const SPACE_CONTROLS = 'button, input, select, textarea, [contenteditable], a, [role="slider"]'

/**
 * The element the mouse (or a touch) gave the focus to, until the focus moves. `:focus-visible` cannot tell
 * this: a browser turns it on at the first key press, Space included, on a button the mouse just clicked.
 */
let pointerFocused: Element | null = null
/** The last device the user acted with: a focus that follows a pointer press (even from a script, in a click handler, or on a window refocus) is the pointer's. */
let lastInput: 'pointer' | 'keyboard' = 'keyboard'
const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'])

/**
 * Starts following where the focus came from: the last input before it was a pointer press (mouse, touch,
 * pen) or a key (Tab, arrows; Space and modifiers do not count). Returns the function that stops it.
 * `spaceTogglesTransport` reads it.
 */
export function trackFocusOrigin(doc: Document = document): () => void {
  const down = (event: Event) => {
    lastInput = 'pointer'
    // A press on the element that already has the focus gives no focus event: it is pointer-focused from now on.
    const active = doc.activeElement
    pointerFocused = active && active !== doc.body && active.contains(event.target as Node | null) ? active : null
  }
  const key = (event: KeyboardEvent) => {
    if (event.code !== 'Space' && !MODIFIER_KEYS.has(event.key)) {
      lastInput = 'keyboard'
      // A UI-shortcut key on an already-focused slider reasserts the keyboard origin even when no
      // focusin fires (e.g. after Slider.closeMenu() → root.focus() while pointer was the last input).
      // Without this, pointerFocused stays stale and Space incorrectly toggles the transport.
      // Scoped to sliders (role="slider"): other controls (buttons, links) may receive shortcut keys
      // whose side effect focuses them back (e.g. Escape on a menu returning focus to its trigger)
      // without meaning that the user switched to keyboard navigation.
      // Checked against BARE_SHORTCUT_CODES (not "!isMusicalKey"): unmapped letters (KeyC, KeyB…)
      // and digits are neither playing keys nor UI shortcuts and must not switch the origin.
      // Note: BARE_SHORTCUT_CODES is compared against event.code; '+', '=', '-', '_' in the list
      // are key values (Equal/Minus are the codes) and will never match, which is correct.
      // Only the modifier keys' own keydowns are excluded above; a chord (Alt+ArrowLeft, Ctrl+Delete) is
      // not a bare shortcut: the slider ignores it, so it must not switch the origin either.
      // Playing keys (KEY_OFFSETS, Z/X) are absent from BARE_SHORTCUT_CODES: a mouse-used knob
      // keeps playing notes even when a musical key is pressed on it.
      if (!event.altKey && !event.ctrlKey && !event.metaKey && (BARE_SHORTCUT_CODES as readonly string[]).includes(event.code)) {
        const active = doc.activeElement
        if (active && active !== doc.body && pointerFocused === active && active.getAttribute('role') === 'slider') {
          pointerFocused = null
        }
      }
    }
  }
  const focus = (event: Event) => {
    pointerFocused = lastInput === 'pointer' ? (event.target as Element) : null
  }
  doc.addEventListener('pointerdown', down, true)
  doc.addEventListener('keydown', key, true)
  doc.addEventListener('focusin', focus, true)
  return () => {
    doc.removeEventListener('pointerdown', down, true)
    doc.removeEventListener('keydown', key, true)
    doc.removeEventListener('focusin', focus, true)
    pointerFocused = null
    lastInput = 'keyboard'
  }
}

/** Whether the keyboard (or a script) put the focus on this control, as opposed to the mouse. */
function keyboardFocused(element: Element): boolean {
  return element !== pointerFocused
}

/**
 * Whether Space toggles the transport (play, or pause at the same position) for a key event aimed at
 * this target. It does everywhere, except where Space already means something:
 * - a field where text is typed, a select or a contenteditable always keeps it, however it got the focus;
 * - any other control (button, link, checkbox, knob, slider) keeps it only when the keyboard put the
 *   focus there (Tab, arrows), so keyboard navigation stays usable. A control the mouse clicked keeps the
 *   focus but not Space, otherwise a click on a tab or a track header would silence the space bar.
 * `focusVisible` is injectable for tests; by default it asks `trackFocusOrigin`.
 */
export function spaceTogglesTransport(target: EventTarget | null, focusVisible: (element: Element) => boolean = keyboardFocused): boolean {
  const element = target as Element | null
  if (!element || typeof element.closest !== 'function') return true
  if (isTextTarget(element)) return false
  const control = element.closest(SPACE_CONTROLS)
  return control === null || !focusVisible(control)
}

/** Where a key means something of its own whoever focused it: a menu (or the button that opened it), a dialog, a spin box (#361). */
const OWN_KEYS_AREAS = '[role="menu"], [role="dialog"], [role="spinbutton"], [aria-haspopup][aria-expanded="true"]'

/**
 * Whether a key event plays a note (or steps the octave) in whatever view is shown (#361). A key is checked against these
 * owners, in this order; the first that claims it wins, and a key nobody claims plays:
 * 1. a key with Ctrl, Cmd or Alt is a shortcut, a repeat is no new note (`ignoreKeyEvent`), and a key a view's handler took
 *    (`preventDefault`) before it reached the window is that view's editing shortcut (no view reads a letter today: this guards a future one);
 * 2. a text field, a select or a contenteditable keeps every key (`ignoreKeyEvent`);
 * 3. a menu (or the button that opened it), a dialog, a spin box keeps every key, however the focus got there, and so does
 *    the whole page while a modal dialog is open;
 * 4. a control the keyboard focused (Tab, arrows: a button, link, checkbox, knob or slider) keeps its own keys, the rule of
 *    Space (`spaceTogglesTransport`); a control the mouse clicked does not, so a sound can be tweaked with the mouse while
 *    notes are played. A control focused by a script after a key (a menu giving the focus back) counts as keyboard-focused.
 *    Exception (#415, option A): a UI-shortcut key (`BARE_SHORTCUT_CODES`: arrows, Delete, Backspace…) pressed on a
 *    mouse-focused slider reasserts keyboard origin (`trackFocusOrigin`), after which letters no longer play on that
 *    slider until the next pointer press. Musical keys (`KEY_OFFSETS`, Z/X) and unmapped letters never change the origin.
 * `focusVisible` is injectable for tests; by default it asks `trackFocusOrigin`.
 */
export function playsNote(
  event: Pick<KeyboardEvent, 'repeat' | 'ctrlKey' | 'metaKey' | 'altKey' | 'target' | 'defaultPrevented'>,
  focusVisible: (element: Element) => boolean = keyboardFocused,
): boolean {
  if (ignoreKeyEvent(event) || event.defaultPrevented) return false
  const element = event.target as Element | null
  if (!element || typeof element.closest !== 'function') return true
  if (element.closest(OWN_KEYS_AREAS) || element.ownerDocument?.querySelector('[aria-modal="true"]')) return false
  const control = element.closest(SPACE_CONTROLS)
  return control === null || !focusVisible(control)
}

/**
 * Save asked by the keyboard (Ctrl or Cmd + S, with Shift for save as), or null. Unlike undo, it also
 * fires inside a text field: there the browser would offer to save the page as a file, which is
 * never what a musician means. It carries Ctrl or Cmd, so it never plays a note.
 */
export function saveShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): 'save' | 'save_as' | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== 's') return null
  return event.shiftKey ? 'save_as' : 'save'
}

/** Undo or redo asked by the keyboard (Ctrl or Cmd + Z, Shift for redo, Ctrl + Y), or null; never inside a text field. */
export function historyShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'target'>): 'undo' | 'redo' | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || isTextTarget(event.target)) return null
  const key = event.key.toLowerCase()
  if (key === 'z') return event.shiftKey ? 'redo' : 'undo'
  if (key === 'y' && !event.shiftKey) return 'redo'
  return null
}
