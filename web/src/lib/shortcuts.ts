/**
 * The shortcut list the "?" button and the `?` key open (#474). The one place that says what each key does for
 * the person reading, kept beside `keyboard.ts`, which says what each key does for the page: a test holds
 * the two together, every bare shortcut of `BARE_SHORTCUT_CODES` and the key of musical typing must be
 * covered by an entry here, so a key added to `keyboard.ts` without a line in this list fails the tests.
 */
import type { MessageKey } from './i18n'
import { SHORTCUT_COMMANDS } from './shortcut-commands'
import { BARE_SHORTCUT_CODES, isTextTarget, NOTE_KEY_CODES, OCTAVE_DOWN_CODE, OCTAVE_UP_CODE, TYPING_TOGGLE_CODE } from './keyboard'

/** A key as shown: `Mod` is Ctrl (Cmd on macOS), `Alt` is Option on macOS, the rest is shown as written. */
export type KeyToken = string

export interface Shortcut {
  /** What it does, in the catalogue. */
  label: MessageKey
  /** Where it works, when not the group's scope: two shortcuts with the same chord in one scope are a conflict (`shortcutConflicts`). */
  scope?: string
  /** The ways to press it: each is a chord (keys held together), alternatives are separate chords. May be empty when the label says the keys itself. */
  chords: readonly (readonly KeyToken[])[]
  /** The `BARE_SHORTCUT_CODES` entries (and the typing key) this line explains. */
  covers?: readonly string[]
  /** The keys the label names, by position, as the layout writes them (the label has `{notes}`, `{rest}`... holes). */
  params?: (layout: KeyLayout | null) => Record<string, string>
  /** Says, after the label, that the keys are a QWERTY keyboard's when the browser does not tell the layout. */
  layoutNote?: boolean
}

export interface ShortcutGroup {
  id: string
  title: MessageKey
  /** Where the group's shortcuts work: `global` is the whole page, any other is the focus it needs (a clip, a lane, a note...). */
  scope: string
  shortcuts: readonly Shortcut[]
}

export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = [
  {
    id: 'general',
    scope: 'global',
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
      { label: 'shortcut.renameProject', chords: [['F2']] },
      {
        label: 'shortcut.views',
        chords: [
          ['Alt', '1'],
          ['Alt', '2'],
          ['Alt', '3'],
        ],
      },
      { label: 'shortcut.panel', chords: [['Alt', 'M']] },
      { label: 'shortcut.tap', chords: [['Alt', 'T']] },
      { label: 'shortcut.browser', chords: [['Alt', 'B']] },
      { label: 'shortcut.help', chords: [['?']] },
      { label: 'shortcut.goTo', chords: [['Mod', 'G']] },
      { label: 'shortcut.palette', chords: [['Mod', 'K']] },
      { label: 'shortcut.tab', chords: [['Tab'], ['Shift', 'Tab']], covers: ['Tab'] },
      { label: 'shortcut.cancel', chords: [['Esc']], covers: ['Escape'] },
    ],
  },
  {
    id: 'notes',
    scope: 'global',
    title: 'shortcut.group.notes',
    shortcuts: [
      { label: 'shortcut.typing', chords: [['`'], ['²']], covers: [TYPING_TOGGLE_CODE] },
      { label: 'shortcut.notes', chords: [], params: (layout) => ({ notes: noteKeysText(layout) }), layoutNote: true },
      { label: 'shortcut.octave', chords: [[OCTAVE_DOWN_CODE], [OCTAVE_UP_CODE]] },
    ],
  },
  {
    id: 'timeline',
    scope: 'timeline.clip',
    title: 'shortcut.group.timeline',
    shortcuts: [
      { label: 'shortcut.tlScroll', scope: 'timeline.view', chords: [['←'], ['→']], covers: ['ArrowLeft', 'ArrowRight'] },
      { label: 'shortcut.tlStart', scope: 'timeline.view', chords: [['Home']], covers: ['Home'] },
      { label: 'shortcut.tlZoom', scope: 'timeline.view', chords: [['+'], ['-']], covers: ['+', '=', '-', '_'] },
      { label: 'shortcut.fitSong', chords: [['Alt', 'Z']] },
      { label: 'shortcut.clipNew', chords: [['Alt', 'N']] },
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
          ['Alt', '↑'],
          ['Alt', '↓'],
        ],
      },
      {
        label: 'shortcut.clipResize',
        chords: [
          ['Alt', 'Shift', '←'],
          ['Alt', 'Shift', '→'],
        ],
      },
      { label: 'shortcut.clipOpen', chords: [['Enter']], covers: ['Enter'] },
      { label: 'shortcut.clipDuplicate', chords: [['Mod', 'D']] },
      { label: 'shortcut.clipDuplicateLinked', chords: [['Mod', 'Shift', 'D']] },
      { label: 'shortcut.clipRepeat', chords: [['Mod', 'Shift', 'R']] },
      // The free cursor (#494): the timeline's and the piano roll's.
      { label: 'shortcut.cursorFree', scope: 'timeline.cursor', chords: [['Esc']] },
      { label: 'shortcut.cursorMove', scope: 'timeline.cursor', chords: [['←'], ['→'], ['↑'], ['↓']] },
      {
        label: 'shortcut.cursorBar',
        scope: 'timeline.cursor',
        chords: [
          ['Shift', '←'],
          ['Shift', '→'],
        ],
      },
      { label: 'shortcut.clipCreate', scope: 'timeline.cursor', chords: [['Enter']] },
      { label: 'shortcut.playheadCursor', scope: 'timeline.ruler', chords: [['Enter'], ['Shift', 'Enter']] },
      {
        label: 'shortcut.markerMove',
        scope: 'timeline.marker',
        chords: [
          ['Alt', '←'],
          ['Alt', '→'],
        ],
      },
      { label: 'shortcut.remove', scope: 'timeline.marker', chords: [['Delete'], ['Backspace']], covers: ['Delete', 'Backspace'] },
      { label: 'shortcut.rename', scope: 'timeline.marker', chords: [['F2']], covers: ['F2'] },
      { label: 'shortcut.laneVolume', scope: 'timeline.lane', chords: [['Enter'], ['0-9']] },
      { label: 'shortcut.laneHeight', scope: 'timeline.lane', chords: [['↑'], ['↓'], ['Home'], ['End']], covers: ['End'] },
      // The automation lanes at the keyboard (#769): their head, then their points as cells of the timeline's grid.
      { label: 'shortcut.lanePoint', scope: 'timeline.lane', chords: [['KeyI'], ['Insert']] },
      { label: 'shortcut.laneRamp', scope: 'timeline.lane', chords: [['KeyR']] },
      { label: 'shortcut.lanePoints', scope: 'timeline.lane', chords: [['→']] },
      { label: 'shortcut.pointStep', scope: 'timeline.point', chords: [['←'], ['→'], ['Home'], ['End']] },
      { label: 'shortcut.pointEdit', scope: 'timeline.point', chords: [['Enter']] },
      {
        label: 'shortcut.pointNudge',
        scope: 'timeline.point',
        chords: [
          ['Alt', '↑'],
          ['Alt', '↓'],
          ['Alt', 'Shift', '↑'],
          ['Alt', 'Shift', '↓'],
          ['Alt', '←'],
          ['Alt', '→'],
        ],
      },
      { label: 'shortcut.pointRemove', scope: 'timeline.point', chords: [['Delete'], ['Backspace']] },
      // A point's curve (#881).
      { label: 'shortcut.pointCurve', scope: 'timeline.point', chords: [['KeyC'], ['Shift', 'KeyC']] },
    ],
  },
  {
    id: 'pianoRoll',
    scope: 'pianoRoll.note',
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
      { label: 'shortcut.noteRepeat', chords: [['Mod', 'Shift', 'R']] },
      { label: 'shortcut.noteGenerate', chords: [['Alt', 'G']] },
      { label: 'shortcut.noteQuantize', chords: [['KeyQ']] },
      { label: 'shortcut.stepEntry', scope: 'pianoRoll.cursor', chords: [['Alt', 'KeyE']] },
      { label: 'shortcut.stepRest', scope: 'pianoRoll.step', chords: [['Period']], params: (layout) => stepKeyParams(layout) },
      { label: 'shortcut.stepLengthen', scope: 'pianoRoll.step', chords: [['Comma']], params: (layout) => stepKeyParams(layout) },
      { label: 'shortcut.stepBack', scope: 'pianoRoll.step', chords: [['Backspace']] },
      {
        label: 'shortcut.noteStep',
        chords: [['←'], ['→'], ['↑'], ['↓'], ['Shift', '←'], ['Shift', '→'], ['Shift', '↑'], ['Shift', '↓']],
      },
      { label: 'shortcut.noteSelect', chords: [['Space'], ['Enter']] },
      {
        label: 'shortcut.noteShift',
        chords: [
          ['Alt', '←'],
          ['Alt', '→'],
        ],
      },
      {
        label: 'shortcut.noteResize',
        chords: [
          ['Alt', 'Shift', '←'],
          ['Alt', 'Shift', '→'],
        ],
      },
      {
        label: 'shortcut.noteTranspose',
        chords: [
          ['Alt', '↑'],
          ['Alt', '↓'],
          ['Alt', 'Shift', '↑'],
          ['Alt', 'Shift', '↓'],
        ],
      },
      { label: 'shortcut.noteCursor', scope: 'pianoRoll.cursor', chords: [['←'], ['→'], ['↑'], ['↓'], ['Shift', '↑'], ['Shift', '↓']] },
      { label: 'shortcut.noteCreate', scope: 'pianoRoll.cursor', chords: [['Enter']] },
      { label: 'shortcut.noteCursorEnds', scope: 'pianoRoll.cursor', chords: [['Home'], ['End']] },
      {
        label: 'shortcut.noteCursorBar',
        scope: 'pianoRoll.cursor',
        chords: [
          ['Mod', '←'],
          ['Mod', '→'],
        ],
      },
      {
        label: 'shortcut.noteVelocity',
        chords: [['Page Up'], ['Page Down'], ['Shift', 'Page Up'], ['Shift', 'Page Down']],
        covers: ['PageUp', 'PageDown'],
      },
    ],
  },
  {
    id: 'grid',
    scope: 'grid',
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
    scope: 'control.slider',
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
      { label: 'shortcut.sliderType', scope: 'control.slider', chords: [['Enter']] },
      {
        label: 'shortcut.tempo',
        scope: 'control.tempo',
        chords: [['↑'], ['↓'], ['Shift', '↑'], ['Shift', '↓'], ['Enter']],
      },
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

/**
 * What the keyboard layout calls the keys, by position: the browser's `navigator.keyboard.getLayoutMap()`
 * (`KeyQ` is "a" on an AZERTY keyboard). A key that works by position (`KeyboardEvent.code`, like the musical typing keys
 * and Q for the quantize) is labelled with the character the user reads on it, never the one the code is named after.
 */
export interface KeyLayout {
  get(code: string): string | undefined
}

/** The layout of the keyboard, or null where the browser cannot say (Firefox, Safari, an insecure page, an error). */
type KeyboardNav = { keyboard?: { getLayoutMap?: () => Promise<KeyLayout> } }
export async function readKeyLayout(nav: KeyboardNav | undefined = globalThis.navigator as unknown as KeyboardNav | undefined): Promise<KeyLayout | null> {
  try {
    return (await nav?.keyboard?.getLayoutMap?.()) ?? null
  } catch {
    return null
  }
}

/** A position-based token (`KeyQ`) as written on the key: the layout's character in capitals, else the letter of the code. */
export function positionKey(code: string, layout: KeyLayout | null = null): string {
  const named = layout?.get(code)
  if (named && named.length === 1) return named.toUpperCase()
  if (code === 'Period') return '.'
  if (code === 'Comma') return ','
  return code.replace(/^Key/, '')
}

/** The keys of the first octave of musical typing as the layout writes them: "A W S E D F T G Y H U J K" on QWERTY, "Q Z S E D F T G Y H U J K" on AZERTY. */
export function noteKeysText(layout: KeyLayout | null): string {
  return NOTE_KEY_CODES.map((code) => positionKey(code, layout)).join(' ')
}

/** What the texts that name the step entry's keys fill their holes with (the "?" list and the step entry's tooltip). */
export function stepKeyParams(layout: KeyLayout | null): Record<string, string> {
  return { rest: positionKey('Period', layout), lengthen: positionKey('Comma', layout) }
}

/** What the Keyboard tab's help fills its holes with: the same keys as the "?" list, from the same functions. */
export function keyHelpParams(layout: KeyLayout | null): Record<string, string> {
  return {
    notes: noteKeysText(layout),
    down: positionKey(OCTAVE_DOWN_CODE, layout),
    up: positionKey(OCTAVE_UP_CODE, layout),
  }
}

/** A key token as the platform writes it. A token `Key<X>` is a key by position, written as the layout names it. */
export function keyLabel(token: KeyToken, mac: boolean = isMac(), layout: KeyLayout | null = null): string {
  if (token === 'Mod') return mac ? 'Cmd' : 'Ctrl'
  if (token === 'Alt') return mac ? 'Option' : 'Alt'
  if (/^(Key[A-Z]|Period|Comma)$/.test(token)) return positionKey(token, layout)
  return token
}

/** A chord as one phrase for tooltips and screen readers: "Ctrl+Shift+Z". */
export function chordText(chord: readonly KeyToken[], mac: boolean = isMac(), layout: KeyLayout | null = null): string {
  return chord.map((token) => keyLabel(token, mac, layout)).join('+')
}

/** A tooltip or label with its shortcut: "Play (Space)". */
export function withShortcut(text: string, chord: readonly KeyToken[], mac: boolean = isMac(), layout: KeyLayout | null = null): string {
  return `${text} (${chordText(chord, mac, layout)})`
}

/** A chord as one comparable string: the modifiers in a fixed order, then the key. `Alt+Shift+←` and `Shift+Alt+←` are one chord. */
export function chordId(chord: readonly KeyToken[]): string {
  const order = ['Mod', 'Alt', 'Shift']
  const mods = order.filter((m) => chord.includes(m))
  const keys = chord.filter((token) => !order.includes(token))
  return [...mods, ...keys].join('+')
}

/** Two shortcuts that press the same chord in the same scope: one key cannot do two things there. */
export interface ShortcutConflict {
  scope: string
  chord: string
  labels: MessageKey[]
}

/** Every chord that two shortcuts of the same scope share (the same chord listed twice by one shortcut counts too). Empty when the registry is consistent (#109). */
export function shortcutConflicts(groups: readonly ShortcutGroup[] = SHORTCUT_GROUPS): ShortcutConflict[] {
  const seen = new Map<string, ShortcutConflict>()
  const conflicts = new Map<string, ShortcutConflict>()
  for (const group of groups) {
    for (const shortcut of group.shortcuts) {
      const scope = shortcut.scope ?? group.scope
      for (const chord of shortcut.chords) {
        const id = chordId(chord)
        const key = `${scope}\u0000${id}`
        const first = seen.get(key)
        if (!first) {
          seen.set(key, { scope, chord: id, labels: [shortcut.label] })
          continue
        }
        first.labels.push(shortcut.label)
        conflicts.set(key, first)
      }
    }
  }
  return [...conflicts.values()]
}

/** The chords the command palette shows for a catalogue command: those of the shortcut `SHORTCUT_COMMANDS` names for it. */
export function commandChords(
  name: string,
  groups: readonly ShortcutGroup[] = SHORTCUT_GROUPS,
  commands: Readonly<Record<string, MessageKey>> = SHORTCUT_COMMANDS,
): (readonly KeyToken[])[] {
  const label = commands[name]
  return label === undefined ? [] : groups.flatMap((g) => g.shortcuts.filter((s) => s.label === label).flatMap((s) => s.chords))
}

/** The keys of a catalogue command as the palette writes them ("Ctrl+Z", chords joined by " / "), or an empty text when no shortcut runs it. */
export function commandShortcutText(
  name: string,
  mac: boolean = isMac(),
  layout: KeyLayout | null = null,
  groups: readonly ShortcutGroup[] = SHORTCUT_GROUPS,
): string {
  return commandChords(name, groups)
    .map((chord) => chordText(chord, mac, layout))
    .join(' / ')
}
