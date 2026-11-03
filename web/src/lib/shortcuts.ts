/**
 * The shortcut list the "?" button and the `?` key open (#474). The one place that says what each key does for
 * the person reading, kept beside `keyboard.ts`, which says what each key does for the page: a test holds
 * the two together, every bare shortcut of `BARE_SHORTCUT_CODES` and the key of musical typing must be
 * covered by an entry here, so a key added to `keyboard.ts` without a line in this list fails the tests.
 */
import type { MessageKey } from './i18n'
import { BARE_SHORTCUT_CODES, isTextTarget, TYPING_TOGGLE_CODE } from './keyboard'

/** A key as shown: `Mod` is Ctrl (Cmd on macOS), `Alt` is Option on macOS, the rest is shown as written. */
export type KeyToken = string

export interface Shortcut {
  /** What it does, in the catalogue. */
  label: MessageKey
  /** The ways to press it: each is a chord (keys held together), alternatives are separate chords. May be empty when the label says the keys itself. */
  chords: readonly (readonly KeyToken[])[]
  /** The `BARE_SHORTCUT_CODES` entries (and the typing key) this line explains. */
  covers?: readonly string[]
}

export interface ShortcutGroup {
  id: string
  title: MessageKey
  shortcuts: readonly Shortcut[]
}

export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = [
  {
    id: 'general',
    title: 'shortcut.group.general',
    shortcuts: [
      { label: 'shortcut.play', chords: [['Space']], covers: ['Space'] },
      { label: 'shortcut.undo', chords: [['Mod', 'Z']] },
      {
        label: 'shortcut.redo',
        chords: [
          ['Mod', 'Shift', 'Z'],
          ['Mod', 'Y'],
        ],
      },
      { label: 'shortcut.save', chords: [['Mod', 'S']] },
      { label: 'shortcut.saveAs', chords: [['Mod', 'Shift', 'S']] },
      {
        label: 'shortcut.views',
        chords: [
          ['Alt', '1'],
          ['Alt', '2'],
          ['Alt', '3'],
        ],
      },
      { label: 'shortcut.panel', chords: [['Alt', 'M']] },
      { label: 'shortcut.help', chords: [['?']] },
      { label: 'shortcut.tab', chords: [['Tab'], ['Shift', 'Tab']], covers: ['Tab'] },
      { label: 'shortcut.cancel', chords: [['Esc']], covers: ['Escape'] },
    ],
  },
  {
    id: 'notes',
    title: 'shortcut.group.notes',
    shortcuts: [
      { label: 'shortcut.typing', chords: [['`'], ['²']], covers: [TYPING_TOGGLE_CODE] },
      { label: 'shortcut.notes', chords: [] },
      { label: 'shortcut.octave', chords: [['Z'], ['X']] },
    ],
  },
  {
    id: 'timeline',
    title: 'shortcut.group.timeline',
    shortcuts: [
      { label: 'shortcut.tlScroll', chords: [['←'], ['→']], covers: ['ArrowLeft', 'ArrowRight'] },
      { label: 'shortcut.tlStart', chords: [['Home']], covers: ['Home'] },
      { label: 'shortcut.tlZoom', chords: [['+'], ['-']], covers: ['+', '=', '-', '_'] },
      { label: 'shortcut.fitSong', chords: [['Alt', 'Z']] },
      { label: 'shortcut.markerAdd', chords: [['Alt', 'K']] },
      {
        label: 'shortcut.markerJump',
        chords: [
          ['Alt', 'Page Up'],
          ['Alt', 'Page Down'],
        ],
      },
      { label: 'shortcut.clipMove', chords: [['←'], ['→'], ['↑'], ['↓']], covers: ['ArrowUp', 'ArrowDown'] },
      {
        label: 'shortcut.clipExtend',
        chords: [
          ['Shift', '←'],
          ['Shift', '→'],
        ],
      },
      {
        label: 'shortcut.clipShift',
        chords: [
          ['Alt', '←'],
          ['Alt', '→'],
        ],
      },
      { label: 'shortcut.clipOpen', chords: [['Enter']], covers: ['Enter'] },
      // The free cursor (#494): the timeline's and the piano roll's.
      { label: 'shortcut.cursorFree', chords: [['Esc']] },
      { label: 'shortcut.cursorMove', chords: [['←'], ['→'], ['↑'], ['↓']] },
      {
        label: 'shortcut.cursorBar',
        chords: [
          ['Shift', '←'],
          ['Shift', '→'],
        ],
      },
      { label: 'shortcut.clipCreate', chords: [['Enter']] },
      { label: 'shortcut.remove', chords: [['Delete'], ['Backspace']], covers: ['Delete', 'Backspace'] },
      { label: 'shortcut.rename', chords: [['F2']], covers: ['F2'] },
      { label: 'shortcut.laneVolume', chords: [['Enter'], ['F2'], ['0-9']] },
      { label: 'shortcut.laneHeight', chords: [['↑'], ['↓'], ['Home'], ['End']], covers: ['End'] },
    ],
  },
  {
    id: 'pianoRoll',
    title: 'shortcut.group.pianoRoll',
    shortcuts: [
      { label: 'shortcut.noteAll', chords: [['Mod', 'A']] },
      {
        label: 'shortcut.noteCopy',
        chords: [
          ['Mod', 'C'],
          ['Mod', 'X'],
          ['Mod', 'V'],
        ],
      },
      { label: 'shortcut.noteDuplicate', chords: [['Mod', 'D']] },
      { label: 'shortcut.noteTranspose', chords: [['↑'], ['↓'], ['Shift', '↑'], ['Shift', '↓']] },
      { label: 'shortcut.noteCursor', chords: [['←'], ['→'], ['↑'], ['↓'], ['Shift', '↑'], ['Shift', '↓']] },
      { label: 'shortcut.noteCreate', chords: [['Enter']] },
      {
        label: 'shortcut.noteVelocity',
        chords: [['Page Up'], ['Page Down'], ['Shift', 'Page Up'], ['Shift', 'Page Down']],
        covers: ['PageUp', 'PageDown'],
      },
    ],
  },
  {
    id: 'grid',
    title: 'shortcut.group.grid',
    shortcuts: [
      { label: 'shortcut.gridMove', chords: [['←'], ['→'], ['↑'], ['↓']] },
      {
        label: 'shortcut.gridCopy',
        chords: [
          ['Mod', 'C'],
          ['Mod', 'V'],
        ],
      },
    ],
  },
  {
    id: 'controls',
    title: 'shortcut.group.controls',
    shortcuts: [
      { label: 'shortcut.sliderStep', chords: [['←'], ['→'], ['↑'], ['↓']] },
      {
        label: 'shortcut.sliderFine',
        chords: [
          ['Mod', '←'],
          ['Mod', '→'],
        ],
      },
      { label: 'shortcut.sliderBig', chords: [['Page Up'], ['Page Down'], ['Home'], ['End']], covers: ['PageUp', 'PageDown'] },
      { label: 'shortcut.sliderDefault', chords: [['Delete']] },
      { label: 'shortcut.sliderType', chords: [['Enter']] },
      { label: 'shortcut.tempo', chords: [['↑'], ['↓'], ['Enter']] },
    ],
  },
]

/** Every `BARE_SHORTCUT_CODES` entry (and the typing key) the list explains: the test compares it with `keyboard.ts`. */
export const COVERED_KEYS: ReadonlySet<string> = new Set(SHORTCUT_GROUPS.flatMap((g) => g.shortcuts.flatMap((s) => s.covers ?? [])))

/** The bare shortcut codes `keyboard.ts` has and the list does not explain: empty when the list is complete. */
export function uncoveredShortcuts(): string[] {
  return [...BARE_SHORTCUT_CODES, TYPING_TOGGLE_CODE].filter((code) => !COVERED_KEYS.has(code))
}

/** Whether a key event asks for the shortcut list: the `?` character, not in a text field, with no Ctrl, Cmd or Alt. It plays nothing (no letter, no Semicolon). */
export function helpShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'repeat' | 'target'>): boolean {
  return event.key === '?' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.repeat && !isTextTarget(event.target)
}

/** Whether the platform is a Mac: its modifier keys are Cmd and Option. */
export function isMac(platform: string = typeof navigator === 'undefined' ? '' : navigator.platform): boolean {
  return /^(Mac|iPhone|iPad|iPod)/i.test(platform)
}

/** A key token as the platform writes it. */
export function keyLabel(token: KeyToken, mac: boolean = isMac()): string {
  if (token === 'Mod') return mac ? 'Cmd' : 'Ctrl'
  if (token === 'Alt') return mac ? 'Option' : 'Alt'
  return token
}

/** A chord as one phrase for tooltips and screen readers: "Ctrl+Shift+Z". */
export function chordText(chord: readonly KeyToken[], mac: boolean = isMac()): string {
  return chord.map((token) => keyLabel(token, mac)).join('+')
}

/** A tooltip or label with its shortcut: "Play (Space)". */
export function withShortcut(text: string, chord: readonly KeyToken[], mac: boolean = isMac()): string {
  return `${text} (${chordText(chord, mac)})`
}
