// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { COMMAND_CATALOG } from './generated/protocol'
import { translate } from './i18n'
import { SHORTCUT_COMMANDS } from './shortcut-commands'
import { BARE_SHORTCUT_CODES, PLAYING_KEYS, TYPING_TOGGLE_CODE } from './keyboard'
import {
  chordId,
  chordText,
  commandChords,
  commandShortcutText,
  helpShortcut,
  keyHelpParams,
  keyLabel,
  noteKeysText,
  positionKey,
  readKeyLayout,
  SHORTCUT_GROUPS,
  SHORTCUT_SCOPES,
  shortcutConflicts,
  stepKeyParams,
  uncoveredShortcuts,
  withShortcut,
} from './shortcuts'

const key = (init: Partial<Parameters<typeof helpShortcut>[0]> & { key: string }) =>
  helpShortcut({ ctrlKey: false, metaKey: false, altKey: false, repeat: false, target: null, ...init })

describe('the shortcut list', () => {
  it('explains every bare shortcut of keyboard.ts and the key of musical typing', () => {
    // A key added to `keyboard.ts` without a line in `shortcuts.ts` shows up here by name.
    expect(uncoveredShortcuts()).toEqual([])
    expect(BARE_SHORTCUT_CODES.length).toBeGreaterThan(0)
    expect(TYPING_TOGGLE_CODE).toBe('Backquote')
  })

  it('has a label in both languages for every line and group, and no chord without a key', () => {
    for (const group of SHORTCUT_GROUPS) {
      for (const lang of ['en', 'fr'] as const) {
        expect(translate(lang, group.title)).not.toBe(group.title)
        for (const shortcut of group.shortcuts) {
          expect(translate(lang, shortcut.label)).not.toBe(shortcut.label)
          for (const chord of shortcut.chords) expect(chord.length).toBeGreaterThan(0)
        }
      }
    }
  })

  it('lists no line twice', () => {
    const labels = SHORTCUT_GROUPS.flatMap((g) => g.shortcuts.map((s) => s.label))
    expect(new Set(labels).size).toBe(labels.length)
  })

  it('never gives a playing key as a bare shortcut chord: a letter plays or it is a shortcut, not both (UI-3)', () => {
    const playing = new Set(PLAYING_KEYS.map((code) => code.replace(/^Key/, '')))
    for (const group of SHORTCUT_GROUPS) {
      for (const shortcut of group.shortcuts) {
        for (const chord of shortcut.chords) {
          // A single letter alone, or with Shift only, would play: the chord needs Mod or Alt, or the letter is the octave (Z, X) or typing's own list.
          if (chord.length === 1 && playing.has(chord[0]!)) {
            expect(['shortcut.octave']).toContain(shortcut.label)
          }
        }
      }
    }
  })

  it('writes the modifiers the platform’s way', () => {
    expect(keyLabel('Mod', false)).toBe('Ctrl')
    expect(keyLabel('Mod', true)).toBe('Cmd')
    expect(keyLabel('Alt', true)).toBe('Option')
    expect(chordText(['Mod', 'Shift', 'Z'], false)).toBe('Ctrl+Shift+Z')
    expect(withShortcut('Play', ['Space'], false)).toBe('Play (Space)')
  })
})

describe('the ? key', () => {
  it('asks for the list on the ? character, with or without Shift held', () => {
    expect(key({ key: '?' })).toBe(true)
  })

  it('does not on another key, with Ctrl, Cmd or Alt, on a repeat, or in a text field', () => {
    expect(key({ key: '/' })).toBe(false)
    expect(key({ key: '?', ctrlKey: true })).toBe(false)
    expect(key({ key: '?', metaKey: true })).toBe(false)
    expect(key({ key: '?', altKey: true })).toBe(false)
    expect(key({ key: '?', repeat: true })).toBe(false)
    const field = document.createElement('input')
    field.type = 'text'
    expect(key({ key: '?', target: field })).toBe(false)
  })
})

describe('the keys of musical typing are named by position, from one source (#906)', () => {
  // The positions that differ on an AZERTY keyboard, as `navigator.keyboard.getLayoutMap()` reports them.
  const azertyMap = new Map([
    ['KeyQ', 'a'],
    ['KeyA', 'q'],
    ['KeyW', 'z'],
    ['KeyZ', 'w'],
    ['Period', ':'],
    ['Comma', ';'],
  ])
  const find = (label: string) => SHORTCUT_GROUPS.flatMap((g) => g.shortcuts).find((s) => s.label === label)!

  it('declares the octave keys by code, so the list writes them as the layout does', () => {
    const octave = find('shortcut.octave')
    expect(octave.chords).toEqual([['KeyZ'], ['KeyX']])
    expect(octave.chords.map(([k]) => keyLabel(k!, false, azertyMap))).toEqual(['W', 'X'])
    expect(octave.chords.map(([k]) => keyLabel(k!, false, null))).toEqual(['Z', 'X'])
  })

  it('builds the notes line from the layout map: Q Z S E D F T G Y H U J K on AZERTY, A W S E D F T G Y H U J K on QWERTY', () => {
    expect(noteKeysText(azertyMap)).toBe('Q Z S E D F T G Y H U J K')
    expect(noteKeysText(null)).toBe('A W S E D F T G Y H U J K')
    const notes = find('shortcut.notes')
    expect(translate('fr', notes.label, notes.params!(azertyMap))).toContain('Q Z S E D F T G Y H U J K jouent')
    expect(translate('en', notes.label, notes.params!(null))).toContain('A W S E D F T G Y H U J K play')
    expect(notes.layoutNote).toBe(true)
    expect(translate('en', 'shortcut.layoutUnknown')).toContain('QWERTY')
    expect(translate('fr', 'shortcut.layoutUnknown')).toContain('QWERTY')
  })

  it('shows the layout character for the rest and lengthen keys of the step entry, never a word', () => {
    expect(stepKeyParams(azertyMap)).toEqual({ rest: ':', lengthen: ';' })
    expect(stepKeyParams(null)).toEqual({ rest: '.', lengthen: ',' })
    for (const lang of ['en', 'fr'] as const) {
      for (const label of ['shortcut.stepRest', 'shortcut.stepLengthen'] as const) {
        expect(translate(lang, label, find(label).params!(azertyMap))).toMatch(/\([:;]\)$/)
      }
      const hint = translate(lang, 'pianoRoll.stepEntryHint', stepKeyParams(azertyMap))
      expect(hint).toContain(':')
      expect(hint).not.toMatch(/\{|point|virgule|Period|Comma/)
    }
  })

  it('gives the Keyboard tab the same keys as the list', () => {
    expect(keyHelpParams(azertyMap)).toEqual({ notes: 'Q Z S E D F T G Y H U J K', down: 'W', up: 'X' })
    expect(keyHelpParams(null)).toEqual({ notes: 'A W S E D F T G Y H U J K', down: 'Z', up: 'X' })
    expect(translate('fr', 'keys.help', keyHelpParams(azertyMap))).toBe(
      'Les touches de la rangée du milieu jouent les notes (par position : Q Z S E D F T G Y H U J K). W / X changent d’octave.',
    )
  })
})

describe('keys by position are written as the layout names them', () => {
  const azerty = new Map([
    ['KeyQ', 'a'],
    ['KeyA', 'q'],
  ])
  it('shows the character on the key: A for the quantize key on AZERTY, Q on QWERTY', () => {
    expect(positionKey('KeyQ', azerty)).toBe('A')
    expect(positionKey('KeyQ', new Map([['KeyQ', 'q']]))).toBe('Q')
    expect(keyLabel('KeyQ', false, azerty)).toBe('A')
    expect(chordText(['Mod', 'KeyQ'], false, azerty)).toBe('Ctrl+A')
    expect(withShortcut('Quantize', ['KeyQ'], false, azerty)).toBe('Quantize (A)')
  })
  it('falls back to the letter of the code when the layout is unknown or gives no single character', () => {
    expect(positionKey('KeyQ', null)).toBe('Q')
    expect(positionKey('KeyQ', new Map())).toBe('Q')
    expect(positionKey('KeyQ', new Map([['KeyQ', 'dead']]))).toBe('Q')
    expect(keyLabel('KeyQ', false)).toBe('Q')
  })
  it('reads the layout map of the browser, and gives null where it is missing or refuses', async () => {
    expect(await readKeyLayout({ keyboard: { getLayoutMap: () => Promise.resolve(azerty) } })).toBe(azerty)
    expect(await readKeyLayout({})).toBeNull()
    expect(await readKeyLayout(undefined)).toBeNull()
    expect(await readKeyLayout({ keyboard: { getLayoutMap: () => Promise.reject(new Error('denied')) } })).toBeNull()
  })
})

describe('the registry has no two shortcuts on one chord in one scope (#109)', () => {
  it('has none today', () => {
    expect(shortcutConflicts()).toEqual([])
  })

  it('finds two shortcuts with the same chord in the same scope, whatever order the modifiers are written in', () => {
    const groups = [
      {
        id: 'a',
        title: 'shortcut.group.general' as const,
        scope: 'global' as const,
        shortcuts: [
          { label: 'shortcut.undo' as const, chords: [['Mod', 'Shift', 'Z']] },
          {
            label: 'shortcut.redo' as const,
            chords: [
              ['Shift', 'Mod', 'Z'],
              ['Mod', 'Y'],
            ],
          },
          { label: 'shortcut.save' as const, chords: [['Mod', 'S']], scope: 'grid' as const, shadows: true },
          { label: 'shortcut.saveAs' as const, chords: [['Mod', 'S']] },
        ],
      },
    ]
    expect(shortcutConflicts(groups)).toEqual([{ scope: 'global', chord: 'Mod+Shift+Z', labels: ['shortcut.undo', 'shortcut.redo'] }])
    expect(chordId(['Shift', 'Alt', '←'])).toBe('Alt+Shift+←')
  })

  it('reports a chord given twice by one shortcut, and each chord once however many shortcuts share it', () => {
    const shortcuts = [
      {
        label: 'shortcut.undo' as const,
        chords: [
          ['Mod', 'Z'],
          ['Mod', 'Z'],
        ],
      },
      { label: 'shortcut.redo' as const, chords: [['Mod', 'Z']] },
    ]
    expect(shortcutConflicts([{ id: 'a', title: 'shortcut.group.general', scope: 'global' as const, shortcuts }])).toEqual([
      { scope: 'global', chord: 'Mod+Z', labels: ['shortcut.undo', 'shortcut.undo', 'shortcut.redo'] },
    ])
  })

  it('names, for each command, a line of the registry and a command of the catalogue', () => {
    const names = new Set(COMMAND_CATALOG.map((e) => e.name))
    const labels = new Set(SHORTCUT_GROUPS.flatMap((g) => g.shortcuts.map((s) => s.label)))
    for (const [command, label] of Object.entries(SHORTCUT_COMMANDS)) {
      expect(names.has(command), command).toBe(true)
      expect(labels.has(label), label).toBe(true)
      expect(commandChords(command).length, command).toBeGreaterThan(0)
    }
  })

  it('only shows keys that work wherever the palette opens: each command’s line is global', () => {
    for (const [command, label] of Object.entries(SHORTCUT_COMMANDS)) {
      const line = SHORTCUT_GROUPS.flatMap((g) => g.shortcuts.map((s) => ({ s, scope: s.scope ?? g.scope }))).filter((x) => x.s.label === label)
      expect(line.length, command).toBe(1)
      expect(line[0]!.scope, command).toBe('global')
    }
  })

  it('has each line once and every scope in the closed list', () => {
    const labels = SHORTCUT_GROUPS.flatMap((g) => g.shortcuts.map((s) => s.label))
    expect(new Set(labels).size).toBe(labels.length)
    for (const group of SHORTCUT_GROUPS) {
      expect(SHORTCUT_SCOPES).toContain(group.scope)
      for (const shortcut of group.shortcuts) if (shortcut.scope) expect(SHORTCUT_SCOPES).toContain(shortcut.scope)
    }
  })

  it('reports a focus line on a global chord unless it says it shadows it, and not two focus scopes on one chord', () => {
    const line = (
      label: 'shortcut.play' | 'shortcut.noteSelect' | 'shortcut.cursorMove',
      scope: 'global' | 'pianoRoll.note' | 'timeline.cursor',
      shadows?: boolean,
    ) => ({
      label,
      chords: [['Space']],
      scope,
      ...(shadows ? { shadows } : {}),
    })
    const group = (shortcuts: ReturnType<typeof line>[]) => [{ id: 'a', title: 'shortcut.group.general' as const, scope: 'global' as const, shortcuts }]
    expect(shortcutConflicts(group([line('shortcut.play', 'global'), line('shortcut.noteSelect', 'pianoRoll.note')]))).toEqual([
      { scope: 'global/pianoRoll.note', chord: 'Space', labels: ['shortcut.play', 'shortcut.noteSelect'] },
    ])
    expect(shortcutConflicts(group([line('shortcut.play', 'global'), line('shortcut.noteSelect', 'pianoRoll.note', true)]))).toEqual([])
    expect(shortcutConflicts(group([line('shortcut.noteSelect', 'pianoRoll.note'), line('shortcut.cursorMove', 'timeline.cursor')]))).toEqual([])
    // Two focus lines of one scope on one global chord are both reported.
    const both = shortcutConflicts(
      group([line('shortcut.play', 'global'), line('shortcut.noteSelect', 'pianoRoll.note'), line('shortcut.cursorMove', 'pianoRoll.note')]),
    )
    expect(both.filter((c) => c.scope.startsWith('global/'))).toHaveLength(2)
  })

  it('gives a line of the registry to at most one command, so a key is shown beside one command only', () => {
    const labels = Object.values(SHORTCUT_COMMANDS)
    expect(new Set(labels).size).toBe(labels.length)
  })
})

describe('the keys the command palette shows beside a command (#109)', () => {
  it('writes the chords of a command the platform way, and nothing for a command no shortcut runs', () => {
    expect(commandChords('history.undo')).toEqual([['Mod', 'Z']])
    expect(commandShortcutText('history.undo', false)).toBe('Ctrl+Z')
    expect(commandShortcutText('history.undo', true)).toBe('Cmd+Z')
    expect(commandShortcutText('project.save_as', false)).toBe('Ctrl+Shift+S')
    expect(commandShortcutText('transport.set_tempo', false)).toBe('')
  })

  it('writes a key by position as the layout names it', () => {
    const groups = [
      {
        id: 'a',
        title: 'shortcut.group.general' as const,
        scope: 'global' as const,
        shortcuts: [{ label: 'shortcut.noteQuantize' as const, chords: [['KeyQ']] }],
      },
    ]
    const commands = { 'clip.notes.quantize': 'shortcut.noteQuantize' as const }
    expect(commandShortcutText('clip.notes.quantize', false, new Map([['KeyQ', 'a']]), groups, commands)).toBe('A')
    expect(commandShortcutText('clip.notes.quantize', false, null, groups, commands)).toBe('Q')
  })
})
