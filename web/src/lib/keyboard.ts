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
 * them) zoom, bare or with Ctrl or Cmd (#471: Ctrl keeps the browser's page zoom off while the grid has the focus). On the selected track's row of the arrangement (#471), Enter or F2 open the field that types its volume, and a digit, a sign or a decimal point opens it with that key typed (`views/arrangement/lane-keys.ts`); the row's height handle moves with Up and Down (Shift for a big step), Home and End, and Enter gives back the default height. Digits play nothing. With a clip of the timeline or a note of the piano roll focused (their named cells, #471, `roving.ts`), the arrows move between them, Shift+arrows extend the selection, Alt+arrows move the selection (Alt+Up and Down transpose notes), Enter opens a clip in the piano roll (on that placement: its origin, play position and entry in the clip list, #494) or sounds a note, and Delete or Backspace removes the selection. Free cursor (#494): the timeline and the piano roll each have a cursor cell (a track and a tick; a pitch and a tick), the only cell in the tab order when they hold no clip or note; Escape on a clip or a note frees the cursor there (Escape again clears the selection); the arrows move it by the snap step, never finer than a beat, in the timeline (the drawing step in the roll) and by a track or a semitone, Shift+Left and Right by a bar (Shift+Up and Down by an octave in the roll); Enter on it creates a clip of one bar on a MIDI track (`clip.create`) or a note of the drawing length (`clip.notes.add`), then focuses it, or focuses the clip or note already under it. Velocity (#494): in the piano roll, Page Up and Page Down raise and lower the velocity of the selection (or of the focused note) by 10, by 1 with Shift, kept in 1–127, as one `clip.notes.update`; the new value is said. Undo and redo (Ctrl or Cmd + Z, Shift or Y) are said in the live regions of the timeline and the piano roll ("Undone: Add notes"). With the launch grid focused, the arrows move between cells, and a scene's name is renamed with F2 (Enter confirms, Escape cancels, as for a track); on a clip, F2 renames it, Delete and Backspace remove it, and Ctrl or Cmd + C then + V copies it into an empty cell. With the piano roll focused, Delete and Backspace remove the selected notes, Escape clears the
 * selection, Ctrl or Cmd + A selects all, Ctrl or Cmd + C, X and V copy, cut and paste notes (pasted at the play position of the clip), Ctrl or Cmd + D duplicates the selection right after itself, and Up and Down transpose it by a semitone (an octave with Shift). Q quantizes the selection (or the whole clip), by position (`isQuantizeKey`): it is the one bare letter that is a shortcut, because it plays nothing and Ctrl + Q belongs to the browser. With a continuous control (`Slider`) focused, the arrows (with Ctrl, Cmd on macOS, for a fine step), Page Up, Page Down, Home and End move it and Delete and Backspace put its default value back (`continuous.ts`), Enter opens a field to type its value (Enter applies, Escape cancels; the field takes every key without Ctrl or Cmd); Tab closes its menu. With the bottom panel's handle focused (a separator), Up and Down resize the panel (Shift for a big step), Home and End go to its smallest and largest size, and Enter folds or unfolds it. The key left of 1 (Backquote, `TYPING_TOGGLE_CODE`) toggles musical typing (#470): with it off, the playing keys play nothing; it is not listed here, because it must not make a mouse-focused slider keyboard-focused. None of them plays a note: the playing keys are letters and Semicolon.
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

/** The key that quantizes in the piano roll (#523): Q by position (`KeyQ`); no modifier, no repeat. It plays nothing (`KEY_OFFSETS` has no Q). */
export const QUANTIZE_CODE = 'KeyQ'
export function isQuantizeKey(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'repeat'>): boolean {
  return event.code === QUANTIZE_CODE && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && !event.repeat
}

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

/** The containers whose children move with the keys (roving grids, menus, radio groups, tabs, toolbars, lists, trees). */
const OWN_KEY_CONTAINERS =
  '[role="grid"], [role="treegrid"], [role="menu"], [role="menubar"], [role="radiogroup"], [role="listbox"], [role="tablist"], [role="toolbar"], [role="tree"]'

/**
 * A plain push button: a `button` with no role of its own (or `button`) outside the containers above. It uses Space and
 * Enter only, so the letters do not belong to it: the engine pill that gave the focus back after its dialog closed
 * must not silence the keyboard (#472). A radio, a tab, a menu item, a cell of a roving grid keeps its keys.
 */
export function isPlainButton(control: Element): boolean {
  if (!control.matches('button')) return false
  const role = control.getAttribute('role')
  if (role !== null && role !== 'button') return false
  return control.closest(OWN_KEY_CONTAINERS) === null
}

/**
 * Whether a key event plays a note (or steps the octave) in whatever view is shown (#361). A key is checked against these
 * owners, in this order; the first that claims it wins, and a key nobody claims plays:
 * 1. a key with Ctrl, Cmd or Alt is a shortcut, a repeat is no new note (`ignoreKeyEvent`), and a key a view's handler took
 *    (`preventDefault`) before it reached the window is that view's editing shortcut (no view reads a letter today: this guards a future one);
 * 2. a text field, a select or a contenteditable keeps every key (`ignoreKeyEvent`);
 * 3. a menu (or the button that opened it), a dialog, a spin box keeps every key, however the focus got there, and so does
 *    the whole page while a modal dialog is open;
 * 4. a control the keyboard focused (Tab, arrows: a link, checkbox, radio, knob or slider) keeps its own keys, the rule of
 *    Space (`spaceTogglesTransport`); a plain push button (`isPlainButton`) does not: it keeps Space and Enter only, so the
 *    letters play on it, whoever focused it (the focus a closed dialog gives back to its button, #472); a control the mouse clicked does not, so a sound can be tweaked with the mouse while
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
  return control === null || isPlainButton(control) || !focusVisible(control)
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

/**
 * "Go to" asked by the keyboard (#554): Ctrl or Cmd + G, in every view, not from a text field (so the open field's own
 * Ctrl+G is left alone). It carries Ctrl or Cmd, so it never plays a note; the browser's find-next is taken.
 */
export function gotoShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'repeat' | 'target'>): boolean {
  return (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && !event.repeat && event.key.toLowerCase() === 'g' && !isTextTarget(event.target)
}

/**
 * The chord of "Repeat until" in the arrangement (#555): Ctrl or Cmd + Shift + R. The browser reloads the page on it, so the
 * arrangement cancels it whenever it is shown, whatever has the focus (a text field and an open dialog included); it opens the
 * field only outside a text field and a dialog.
 */
export function repeatShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): boolean {
  return (event.ctrlKey || event.metaKey) && event.shiftKey && !event.altKey && event.key.toLowerCase() === 'r'
}

/** Undo or redo asked by the keyboard (Ctrl or Cmd + Z, Shift for redo, Ctrl + Y), or null; never inside a text field. */
export function historyShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'target'>): 'undo' | 'redo' | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || isTextTarget(event.target)) return null
  const key = event.key.toLowerCase()
  if (key === 'z') return event.shiftKey ? 'redo' : 'undo'
  if (key === 'y' && !event.shiftKey) return 'redo'
  return null
}

/**
 * The layout shortcuts (#472), all with Alt alone (never Ctrl: the browser takes Ctrl+digit for its tabs; never a bare
 * letter: the keys play). By position, so the layout does not matter: Alt+1, Alt+2, Alt+3 show the main views in the
 * registry's order (Arrangement, Grid, Mixer), Alt+M maximises the bottom panel or gives it back its size. Not in a
 * text field. Playing ignores any Alt event (`ignoreKeyEvent`), so none of these plays a note.
 */
const PANEL_CODE = 'KeyM'
/**
 * The arrangement's keys for its sections and its zoom (#473): Alt+K adds a section marker at the playhead's bar,
 * Alt+Page Up and Alt+Page Down jump to the previous and the next marker, Alt+Z shows the whole song. Never bare
 * letters (they play notes in musical typing); nothing from a text field.
 */
export function arrangementShortcut(
  event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'target'>,
): 'addMarker' | 'prevMarker' | 'nextMarker' | 'fitSong' | null {
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || isTextTarget(event.target)) return null
  return ARRANGEMENT_CODES[event.code] ?? null
}

/** The codes of `arrangementShortcut`, always with Alt (so not bare shortcuts). */
const ARRANGEMENT_CODES: Readonly<Record<string, 'addMarker' | 'prevMarker' | 'nextMarker' | 'fitSong'>> = {
  KeyK: 'addMarker',
  PageUp: 'prevMarker',
  PageDown: 'nextMarker',
  KeyZ: 'fitSong',
}

export function layoutShortcut(
  event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'repeat' | 'target'>,
): { view: number } | 'panel' | null {
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.repeat || isTextTarget(event.target)) return null
  const digit = /^(?:Digit|Numpad)([1-9])$/.exec(event.code)
  if (digit) return { view: Number(digit[1]) - 1 }
  return event.code === PANEL_CODE ? 'panel' : null
}

/**
 * The device browser (#190): Alt+B, by position. Alt alone (never a bare letter: B plays no note, but the rule is the
 * same for every shortcut), no repeat, nothing from a text field except the browser's own search field, where the
 * same key closes it. Playing ignores every Alt event (`ignoreKeyEvent`), so it plays nothing.
 */
const BROWSER_CODE = 'KeyB'
export function browserShortcut(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'repeat' | 'target'>): boolean {
  if (event.code !== BROWSER_CODE || !event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.repeat) return false
  const target = event.target as HTMLElement | null
  return !isTextTarget(target) || (typeof target?.closest === 'function' && target.closest('[data-browser-search]') !== null)
}

/**
 * Tap tempo (#195): Alt+T, by position. Alt alone (never a bare letter: T plays a note), no repeat (a held key is one tap),
 * nothing from a text field. Playing ignores every Alt event (`ignoreKeyEvent`), so it plays nothing.
 */
const TAP_CODE = 'KeyT'
export function tapShortcut(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'repeat' | 'target'>): boolean {
  return isTapChord(event) && !event.repeat && !isTextTarget(event.target)
}

/** The tap chord itself, repeats and text fields included: what a series of taps must not mistake for another key. */
export function isTapChord(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): boolean {
  return event.code === TAP_CODE && event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey
}
