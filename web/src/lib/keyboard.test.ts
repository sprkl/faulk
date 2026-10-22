// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { BARE_SHORTCUT_CODES, clampOctave, defaultOctave, historyShortcut, ignoreKeyEvent, noteForKey, octaveStep, PLAYING_KEYS } from './keyboard'

describe('computer keyboard', () => {
  it('maps physical keys to notes', () => {
    expect(noteForKey('KeyA', 4)).toBe(60)
    expect(noteForKey('KeyW', 4)).toBe(61)
    expect(noteForKey('KeyK', 4)).toBe(72)
    expect(noteForKey('KeyA', 0)).toBe(12)
    expect(noteForKey('KeyQ', 4)).toBeNull()
    expect(noteForKey('Semicolon', 9)).toBeNull() // above 127
  })

  it('shifts and clamps octaves', () => {
    expect(octaveStep('KeyZ')).toBe(-1)
    expect(octaveStep('KeyX')).toBe(1)
    expect(octaveStep('KeyA')).toBe(0)
    expect(clampOctave(-2)).toBe(0)
    expect(clampOctave(12)).toBe(8)
  })

  it('ignores repeats, shortcuts and typing in fields', () => {
    const base = { repeat: false, ctrlKey: false, metaKey: false, altKey: false, target: document.body }
    expect(ignoreKeyEvent(base)).toBe(false)
    expect(ignoreKeyEvent({ ...base, repeat: true })).toBe(true)
    expect(ignoreKeyEvent({ ...base, ctrlKey: true })).toBe(true)
    const input = document.createElement('input')
    document.body.append(input)
    expect(ignoreKeyEvent({ ...base, target: input })).toBe(true)
    const field = document.createElement('div')
    field.innerHTML = '<label><input type="text"></label><textarea></textarea>'
    expect(ignoreKeyEvent({ ...base, target: field.querySelector('input') })).toBe(true)
    expect(ignoreKeyEvent({ ...base, target: field.querySelector('textarea') })).toBe(true)
  })

  it('keeps playing while a knob, button, radio or checkbox has the focus', () => {
    const base = { repeat: false, ctrlKey: false, metaKey: false, altKey: false }
    const box = document.createElement('div')
    box.innerHTML = '<input type="range"><input type="radio"><input type="checkbox"><button>b</button>'
    for (const el of box.children) expect(ignoreKeyEvent({ ...base, target: el }), el.outerHTML).toBe(false)
  })
})

describe('history shortcuts', () => {
  const base = { key: 'z', ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, target: document.body }

  it('maps Ctrl or Cmd + Z to undo, Shift + Z and Ctrl + Y to redo', () => {
    expect(historyShortcut(base)).toBe('undo')
    expect(historyShortcut({ ...base, ctrlKey: false, metaKey: true })).toBe('undo')
    expect(historyShortcut({ ...base, key: 'Z', shiftKey: true })).toBe('redo')
    expect(historyShortcut({ ...base, key: 'y' })).toBe('redo')
  })

  it('leaves the other keys, a plain Z and text fields alone', () => {
    expect(historyShortcut({ ...base, ctrlKey: false })).toBeNull()
    expect(historyShortcut({ ...base, key: 'a' })).toBeNull()
    expect(historyShortcut({ ...base, altKey: true })).toBeNull()
    expect(historyShortcut({ ...base, key: 'y', shiftKey: true })).toBeNull()
    const input = document.createElement('input')
    input.type = 'text'
    document.body.append(input)
    expect(historyShortcut({ ...base, target: input })).toBeNull()
    const range = document.createElement('input')
    range.type = 'range'
    expect(historyShortcut({ ...base, target: range })).toBe('undo')
  })
})

describe('playing and shortcuts never meet (UI-3)', () => {
  const letter = (code: string) => (code === 'Semicolon' ? ';' : code.replace('Key', '').toLowerCase())
  const bare = { repeat: false, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, target: document.body }

  it('plays no note from a key that fires a shortcut: every playing key, bare, fires none', () => {
    expect(PLAYING_KEYS.length).toBe(17)
    for (const code of [...PLAYING_KEYS, 'KeyZ', 'KeyX']) {
      expect(historyShortcut({ ...bare, key: letter(code) }), code).toBeNull()
      expect(historyShortcut({ ...bare, shiftKey: true, key: letter(code).toUpperCase() }), code).toBeNull()
    }
  })

  it('plays nothing while a shortcut is held: every playing key with Ctrl, Cmd or Alt is ignored', () => {
    for (const code of [...PLAYING_KEYS, 'KeyZ', 'KeyX']) {
      for (const held of ['ctrlKey', 'metaKey', 'altKey'] as const) {
        expect(ignoreKeyEvent({ ...bare, code, key: letter(code), [held]: true } as never), `${code} ${held}`).toBe(true)
      }
    }
    // The shortcut itself still works, and it is the undo, not a note nor an octave step.
    expect(historyShortcut({ ...bare, ctrlKey: true, key: 'z' })).toBe('undo')
  })

  it('starts drums where their sounds are (notes 36 to 46 on the key row), anything else at octave 4', () => {
    const octave = defaultOctave('faulk.drums')
    const notes = PLAYING_KEYS.map((code) => noteForKey(code, octave))
    for (const sound of [36, 38, 39, 42, 46]) expect(notes, `note ${sound}`).toContain(sound)
    expect(noteForKey('KeyA', octave)).toBe(36)
    expect(defaultOctave('faulk.osc_synth')).toBe(4)
    expect(defaultOctave(null)).toBe(4)
  })

  it('keeps the bare shortcuts off the playing keys, and knows every component that handles keys', () => {
    for (const code of BARE_SHORTCUT_CODES) {
      expect(PLAYING_KEYS, code).not.toContain(code)
      expect(octaveStep(code), code).toBe(0)
    }
    // Every component with a key handler is listed here: a new one must follow the rule of keyboard.ts
    // (no bare letter) and be added, with the keys it reads, to this table and to BARE_SHORTCUT_CODES.
    const sources = import.meta.glob(['/src/**/*.svelte', '/src/**/*.ts', '!/src/**/*.test.ts', '!/src/lib/generated/**'], {
      query: '?raw',
      import: 'default',
      eager: true,
    }) as Record<string, string>
    const handlers: Record<string, string[]> = {}
    for (const [file, text] of Object.entries(sources)) {
      if (!/\bon(keydown|keyup)\b|addEventListener\(\s*'key(down|up)'/.test(text)) continue
      // Every literal a handler compares a key or a code with: `=== 'x'`, `!== 'x'`, `case 'x':`.
      const reads = [...text.matchAll(/(?:\.(?:key|code)\s*[!=]==\s*|bcase\s+)'([^']+)'/g)].map((m) => m[1]!)
      handlers[file.replace('/src/', '')] = reads.sort()
    }
    expect(handlers).toEqual({
      'lib/components/timegrid/TimeGrid.svelte': ['+', '-', '=', 'ArrowLeft', 'ArrowRight', 'Home', '_'],
      'lib/components/TrackHeader.svelte': ['Enter', 'Enter', 'Escape', 'F2'],
      'views/play/PlayView.svelte': [],
      'views/shell/Shell.svelte': [],
      'views/tracks/TrackList.svelte': ['Escape'],
      'views/transport/Transport.svelte': ['ArrowDown', 'ArrowUp', 'ArrowUp', 'Enter', 'Enter', 'Escape', 'Escape', 'Space'],
    })
    const bare = new Set(Object.values(handlers).flat())
    for (const key of bare) expect(BARE_SHORTCUT_CODES, key).toContain(key)
  })
})
