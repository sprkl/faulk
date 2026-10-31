// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import {
  BARE_SHORTCUT_CODES,
  clampOctave,
  defaultOctave,
  historyShortcut,
  ignoreKeyEvent,
  noteForKey,
  octaveStep,
  PLAYING_KEYS,
  playsNote,
  saveShortcut,
  spaceTogglesTransport,
  trackFocusOrigin,
} from './keyboard'

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
    box.innerHTML = '<div role="slider" tabindex="0"></div><input type="range"><input type="radio"><input type="checkbox"><button>b</button>'
    for (const el of box.children) expect(ignoreKeyEvent({ ...base, target: el }), el.outerHTML).toBe(false)
  })
})

describe('space bar and the transport', () => {
  const box = document.createElement('div')
  box.innerHTML =
    '<button id="b">b</button><a id="a" href="#">a</a><input id="t" type="text"><input id="n" type="number"><input id="c" type="checkbox"><input id="r" type="range">' +
    '<textarea id="x"></textarea><select id="s"><option>o</option></select><div id="e" contenteditable="true"></div><div id="k" role="slider" tabindex="0"></div>' +
    '<div id="g" tabindex="0"></div><button id="w"><span id="in">i</span></button>'
  document.body.append(box)
  const el = (id: string) => box.querySelector('#' + id)!
  const mouse = () => false
  const keyboard = () => true

  it('toggles from the page, a mouse-clicked control or a plain focusable area', () => {
    expect(spaceTogglesTransport(null, keyboard)).toBe(true)
    expect(spaceTogglesTransport(document.body, keyboard)).toBe(true)
    expect(spaceTogglesTransport(el('g'), keyboard)).toBe(true)
    for (const id of ['b', 'a', 'c', 'r', 'k', 'in']) expect(spaceTogglesTransport(el(id), mouse), id).toBe(true)
  })

  it('leaves Space to a control the keyboard focused', () => {
    for (const id of ['b', 'a', 'c', 'r', 'k', 'in']) expect(spaceTogglesTransport(el(id), keyboard), id).toBe(false)
  })

  it('always leaves Space to text fields, selects and contenteditable, however they got the focus', () => {
    for (const id of ['t', 'n', 'x', 's', 'e']) {
      expect(spaceTogglesTransport(el(id), mouse), id).toBe(false)
      expect(spaceTogglesTransport(el(id), keyboard), id).toBe(false)
    }
  })

  it('follows where the focus came from: a pressed control is pointer-focused, a Tab or script focus is keyboard-focused', () => {
    const stop = trackFocusOrigin()
    const press = (target: Element) => {
      target.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
      ;(target as HTMLElement).focus()
      target.dispatchEvent(new MouseEvent('pointerup', { bubbles: true }))
    }
    const tab = () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', code: 'Tab', bubbles: true }))
    const b = el('b') as HTMLElement
    const a = el('a') as HTMLElement
    press(b)
    expect(spaceTogglesTransport(b)).toBe(true)
    // Space and modifiers do not turn the origin into the keyboard's.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Shift', code: 'ShiftLeft', bubbles: true }))
    document.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true }))
    b.blur()
    b.focus()
    expect(spaceTogglesTransport(b)).toBe(true)
    // A focus a click handler gives by script (a menu giving it back), or the window giving it back, is still the pointer's.
    a.focus()
    expect(spaceTogglesTransport(a)).toBe(true)
    // Tab: the keyboard's.
    tab()
    b.focus()
    expect(spaceTogglesTransport(b)).toBe(false)
    // A press on the control that already has the focus makes it pointer-focused again.
    b.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(true)
    tab()
    a.focus()
    expect(spaceTogglesTransport(a)).toBe(false)
    stop()
    b.focus()
    expect(spaceTogglesTransport(b)).toBe(false)
  })

  it('a UI-shortcut key on the already-focused element reasserts keyboard origin (#415)', () => {
    // Regression: Slider.closeMenu() calls root.focus() while lastInput === 'pointer'. If the element
    // is already focused a subsequent focus() is a no-op (no focusin), leaving pointerFocused stale.
    // A non-playing UI key on that element must clear it so Space does not incorrectly toggle.
    const stop = trackFocusOrigin()
    const b = el('b') as HTMLElement
    // Press (pointer) and focus: b becomes pointer-focused.
    b.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    b.focus()
    expect(spaceTogglesTransport(b)).toBe(true) // pointer-focused: space toggles

    // Space and modifiers must NOT reassert keyboard origin.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Shift', code: 'ShiftLeft', bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(true)
    document.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(true)

    // Playing keys (bare letters, octave steps Z/X) do NOT reassert keyboard origin:
    // a mouse-used knob must keep playing notes when the musician presses a key.
    b.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(true) // still pointer-focused
    b.dispatchEvent(new KeyboardEvent('keydown', { key: 'z', code: 'KeyZ', bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(true) // octave step, still pointer-focused

    // A UI-shortcut key (Delete, Backspace, arrows, etc.) reasserts keyboard origin.
    b.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', code: 'Delete', bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(false) // keyboard origin now

    // b.focus() while already focused is a no-op: keyboard origin is preserved.
    b.focus()
    expect(spaceTogglesTransport(b)).toBe(false)
    // Space does not toggle (keyboard-focused).
    document.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(false)

    // A pointer press restores pointer origin.
    b.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(true)
    stop()
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

describe('save shortcuts', () => {
  const base = { key: 's', ctrlKey: true, metaKey: false, altKey: false, shiftKey: false }

  it('maps Ctrl or Cmd + S to save and Shift + S to save as', () => {
    expect(saveShortcut(base)).toBe('save')
    expect(saveShortcut({ ...base, ctrlKey: false, metaKey: true })).toBe('save')
    expect(saveShortcut({ ...base, key: 'S', shiftKey: true })).toBe('save_as')
  })

  it('leaves a plain S (it plays), Alt and the other letters alone', () => {
    expect(saveShortcut({ ...base, ctrlKey: false })).toBeNull()
    expect(saveShortcut({ ...base, altKey: true })).toBeNull()
    expect(saveShortcut({ ...base, key: 'a' })).toBeNull()
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
      expect(saveShortcut({ ...bare, key: letter(code) }), code).toBeNull()
      expect(saveShortcut({ ...bare, shiftKey: true, key: letter(code).toUpperCase() }), code).toBeNull()
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
    expect(saveShortcut({ ...bare, ctrlKey: true, key: 's' })).toBe('save')
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
      if (!/\bon(keydown|keyup)\b|addEventListener\(\s*'key(down|up)'|export function keyAction\b/.test(text)) continue
      // Every literal a handler compares a key or a code with: `=== 'x'`, `!== 'x'`, `case 'x':`.
      const reads = [...text.matchAll(/(?:\.(?:key|code)\s*[!=]==\s*|\bcase\s+)'([^']+)'/g)].map((m) => m[1]!)
      handlers[file.replace('/src/', '')] = reads.sort()
    }
    expect(handlers).toEqual({
      'lib/keyboard.ts': ['Space'], // trackFocusOrigin only notes that a key was pressed; it triggers nothing
      'lib/components/Slider.svelte': ['Enter'], // opens the value field (#365)
      'lib/components/ValueField.svelte': ['Enter', 'Escape'],
      'lib/components/ValueMenu.svelte': ['Escape', 'Tab'],
      'lib/continuous.ts': ['ArrowDown', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'Backspace', 'Delete', 'End', 'Home', 'PageDown', 'PageUp'],
      'lib/components/timegrid/TimeGrid.svelte': ['+', '-', '=', 'ArrowLeft', 'ArrowRight', 'Home', '_'],
      'lib/components/TrackHeader.svelte': ['Enter', 'Enter', 'Escape', 'Escape', 'F2'],
      'views/arrangement/ArrangementView.svelte': ['Backspace', 'Delete', 'Escape'],
      'views/grid/GridView.svelte': [
        'ArrowDown',
        'ArrowLeft',
        'ArrowRight',
        'ArrowUp',
        'Backspace',
        'Delete',
        'Enter',
        'Enter',
        'Enter',
        'Enter',
        'Escape',
        'Escape',
        'Escape',
        'Escape',
        'Escape',
        'F2',
        'F2',
      ],
      'views/piano-roll/PianoRollView.svelte': ['ArrowDown', 'ArrowUp', 'ArrowUp', 'Backspace', 'Delete', 'Enter', 'Enter', 'Escape', 'Escape', 'F2'],
      'views/shell/BottomPanel.svelte': ['ArrowDown', 'ArrowUp', 'End', 'Enter', 'Home'],
      'views/shell/ExportDialog.svelte': ['Escape'],
      'views/shell/ProjectMenu.svelte': ['Escape'],
      'views/shell/Shell.svelte': [],
      'views/shell/StorageBanners.svelte': ['Escape'],
      'views/tracks/TrackList.svelte': ['Escape'],
      'views/transport/Transport.svelte': ['ArrowDown', 'ArrowUp', 'ArrowUp', 'Enter', 'Enter', 'Escape', 'Escape', 'Space'],
    })
    const bare = new Set(Object.values(handlers).flat())
    for (const key of bare) expect(BARE_SHORTCUT_CODES, key).toContain(key)
  })
})

describe('the keys are shared in every view (#361)', () => {
  const event = (over: Record<string, unknown>) =>
    ({ repeat: false, ctrlKey: false, metaKey: false, altKey: false, defaultPrevented: false, target: document.body, ...over }) as never
  const fixture = (html: string) => {
    document.body.innerHTML = html
    return document.body.firstElementChild!
  }

  it('1. a text field keeps every key, for every key that plays', () => {
    for (const html of [
      '<input type="text">',
      '<input type="number">',
      '<input type="search">',
      '<textarea></textarea>',
      '<select></select>',
      '<div contenteditable="true"></div>',
    ]) {
      expect(playsNote(event({ target: fixture(html) })), html).toBe(false)
    }
  })

  it('1b. a menu, a dialog and a spin box keep every key, whoever focused them', () => {
    for (const html of [
      '<div role="menu"><button role="menuitem"></button></div>',
      '<div role="dialog"><button></button></div>',
      '<div role="spinbutton" tabindex="0"></div>',
      '<button aria-haspopup="menu" aria-expanded="true"></button>',
    ]) {
      const root = fixture(html)
      const inner = root.firstElementChild ?? root
      for (const target of [root, inner])
        expect(
          playsNote(event({ target }), () => false),
          html,
        ).toBe(false)
    }
  })

  it('1c. a modal dialog keeps every key of the page, even when the focus fell back to the body', () => {
    fixture('<div role="dialog" aria-modal="true"></div><button></button>')
    expect(playsNote(event({ target: document.body }))).toBe(false)
    expect(playsNote(event({ target: document.querySelector('button') }), () => false)).toBe(false)
    document.body.innerHTML = ''
    expect(playsNote(event({ target: document.body }))).toBe(true)
  })

  it('2. a control the keyboard focused keeps its keys, one the mouse clicked does not', () => {
    for (const html of [
      '<button></button>',
      '<a href="#"></a>',
      '<input type="checkbox">',
      '<input type="radio">',
      '<input type="range">',
      '<div role="slider" tabindex="0"></div>',
    ]) {
      const target = fixture(html)
      expect(
        playsNote(event({ target }), () => true),
        `${html} focused by the keyboard`,
      ).toBe(false)
      expect(
        playsNote(event({ target }), () => false),
        `${html} focused by the mouse`,
      ).toBe(true)
    }
    // Not a control: a view's own surface (a time grid, a panel) plays whoever focused it.
    expect(playsNote(event({ target: fixture('<div tabindex="0"></div>') }), () => true)).toBe(true)
  })

  it('3. a view that took the key keeps it, and a modifier makes a shortcut, not a note', () => {
    expect(playsNote(event({ defaultPrevented: true }))).toBe(false)
    for (const held of ['ctrlKey', 'metaKey', 'altKey']) expect(playsNote(event({ [held]: true })), held).toBe(false)
    expect(playsNote(event({ repeat: true }))).toBe(false)
  })

  it('4. otherwise the key plays: on the page, on a view or on a clicked control', () => {
    expect(playsNote(event({}))).toBe(true)
    expect(playsNote(event({ target: fixture('<div tabindex="0" role="application"></div>') }))).toBe(true)
  })
})
