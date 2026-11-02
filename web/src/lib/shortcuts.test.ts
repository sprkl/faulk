// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { translate } from './i18n'
import { BARE_SHORTCUT_CODES, PLAYING_KEYS, TYPING_TOGGLE_CODE } from './keyboard'
import { chordText, helpShortcut, keyLabel, SHORTCUT_GROUPS, uncoveredShortcuts, withShortcut } from './shortcuts'

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
