// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { translate } from './i18n'
import { BARE_SHORTCUT_CODES, PLAYING_KEYS, TYPING_TOGGLE_CODE } from './keyboard'
import {
  chordText,
  helpShortcut,
  keyHelpParams,
  keyLabel,
  noteKeysText,
  positionKey,
  readKeyLayout,
  SHORTCUT_GROUPS,
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
