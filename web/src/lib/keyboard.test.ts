// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
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
import { isPlainButton, layoutShortcut } from './keyboard'

// Trackers started by a test are stopped even when it fails, so module state never leaks into the next one.
const trackers: Array<() => void> = []
afterEach(() => {
  for (const stop of trackers.splice(0)) stop()
})

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
    trackers.push(stop)
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

  it('a UI-shortcut key on an already-focused slider reasserts keyboard origin (#415)', () => {
    // Regression: Slider.closeMenu() calls root.focus() while lastInput === 'pointer'. The
    // real path of reassertion is the pointerdown handler 'down' on the already-focused control
    // (e.g. the right-click that opens the menu): it sets pointerFocused = active. A subsequent
    // focus() call on the already-focused element is a no-op (no focusin fires), so pointerFocused
    // stays stale. A non-musical UI-shortcut key on the focused slider must clear pointerFocused.
    // Scoped to sliders (role="slider"): other controls (buttons etc.) are left alone so that e.g.
    // pressing Escape on a menu that returns focus to its trigger does not change the focus model.
    const stop = trackFocusOrigin()
    trackers.push(stop)
    const k = el('k') as HTMLElement // k has role="slider"

    // Press (pointer) and focus the slider: it becomes pointer-focused.
    k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    k.focus()
    expect(spaceTogglesTransport(k)).toBe(true) // pointer-focused: space toggles

    // Space and modifiers must NOT reassert keyboard origin.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Shift', code: 'ShiftLeft', bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(true)
    document.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true }))
    // A chord (Alt+ArrowLeft, Ctrl+Delete, Cmd+Backspace) is not a bare shortcut: the origin stays pointer.
    for (const mod of ['altKey', 'ctrlKey', 'metaKey'] as const) {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', code: 'Delete', [mod]: true, bubbles: true }))
      expect(spaceTogglesTransport(k)).toBe(true)
    }
    expect(spaceTogglesTransport(k)).toBe(true)

    // Playing keys (bare letters, octave steps Z/X) do NOT reassert keyboard origin:
    // a mouse-used knob must keep playing notes when the musician presses a key.
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(true) // still pointer-focused
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'z', code: 'KeyZ', bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(true) // octave step, still pointer-focused

    // Unmapped letters (KeyC, KeyB…) and digits are neither playing keys nor BARE_SHORTCUT_CODES:
    // they must not reassert keyboard origin either.
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'c', code: 'KeyC', bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(true) // still pointer-focused
    k.dispatchEvent(new KeyboardEvent('keydown', { key: '1', code: 'Digit1', bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(true) // still pointer-focused

    // Every BARE_SHORTCUT_CODES entry that is a real event.code reasserts keyboard origin on a
    // focused slider. Enumerated from the exported list so "every" is proven. Space is excluded by
    // the outer guard; '+', '=', '-', '_' are key values, never codes (tested dead in the next test).
    const KEY_ONLY = new Set(['+', '=', '-', '_'])
    const shortcutCodes = BARE_SHORTCUT_CODES.filter((c) => c !== 'Space' && !KEY_ONLY.has(c))
    expect(shortcutCodes).toEqual([
      'F2',
      'Escape',
      'Enter',
      'ArrowUp',
      'ArrowDown',
      'ArrowLeft',
      'ArrowRight',
      'Home',
      'Delete',
      'Backspace',
      'PageUp',
      'PageDown',
      'End',
      'Tab',
    ])
    for (const code of shortcutCodes) {
      k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
      k.focus()
      expect(spaceTogglesTransport(k), `${code} before`).toBe(true) // pointer-focused
      k.dispatchEvent(new KeyboardEvent('keydown', { key: code, code, bubbles: true }))
      expect(spaceTogglesTransport(k), `${code} after`).toBe(false) // keyboard origin now
    }

    // Restore pointer origin for subsequent assertions.
    k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    k.focus()
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', code: 'Delete', bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(false) // keyboard origin now

    // k.focus() while already focused is a no-op: keyboard origin is preserved.
    k.focus()
    expect(spaceTogglesTransport(k)).toBe(false)
    // Space does not toggle (keyboard-focused).
    document.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(false)

    // A pointer press restores pointer origin.
    k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    expect(spaceTogglesTransport(k)).toBe(true)

    // Non-slider controls (buttons, links) are NOT affected: pressing a shortcut key on a button
    // does not change its focus model (e.g. Escape on a menu trigger should not make it keyboard-focused).
    const b = el('b') as HTMLElement
    b.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    b.focus()
    expect(spaceTogglesTransport(b)).toBe(true) // pointer-focused
    b.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', code: 'Delete', bubbles: true }))
    expect(spaceTogglesTransport(b)).toBe(true) // still pointer-focused (button, not slider)
    stop()
  })

  it('after a non-musical key on a mouse-focused slider, letters no longer play notes (#415 option A)', () => {
    // Option A (lead decision): focus origin follows the last input modality. A non-musical key
    // (Delete, arrows…) on a slider that the mouse focused switches it to keyboard origin: letters
    // no longer play a note on that slider until the next click restores pointer origin.
    // This is the visible consequence of the fix: not just Space, but playsNote changes too.
    const stop = trackFocusOrigin()
    trackers.push(stop)
    const k = el('k') as HTMLElement // role="slider"
    const base = { repeat: false, ctrlKey: false, metaKey: false, altKey: false, defaultPrevented: false }

    // Pointer-focused: letter on the slider plays (mouse-used knob does not capture keys).
    k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    k.focus()
    expect(playsNote({ ...base, target: k })).toBe(true) // pointer origin: plays

    // Non-musical key → keyboard origin → letters no longer play on that slider.
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', code: 'Delete', bubbles: true }))
    expect(playsNote({ ...base, target: k })).toBe(false) // keyboard origin: slider keeps letters

    // Arrow key confirms: still keyboard origin.
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', code: 'ArrowUp', bubbles: true }))
    expect(playsNote({ ...base, target: k })).toBe(false)

    // A click (pointer press) restores pointer origin: letters play again.
    k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    expect(playsNote({ ...base, target: k })).toBe(true) // pointer origin restored

    // Playing keys (KeyA, KeyZ octave step) never change the origin: a mouse-used knob stays pointer-focused.
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', bubbles: true }))
    expect(playsNote({ ...base, target: k })).toBe(true) // still pointer-focused
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'z', code: 'KeyZ', bubbles: true }))
    expect(playsNote({ ...base, target: k })).toBe(true)

    // Unmapped letters (KeyC) and digits (Digit1) do not change the origin either.
    k.dispatchEvent(new KeyboardEvent('keydown', { key: 'c', code: 'KeyC', bubbles: true }))
    expect(playsNote({ ...base, target: k })).toBe(true) // still pointer-focused: KeyC is not a shortcut
    k.dispatchEvent(new KeyboardEvent('keydown', { key: '1', code: 'Digit1', bubbles: true }))
    expect(playsNote({ ...base, target: k })).toBe(true) // still pointer-focused: Digit1 is not a shortcut

    // Every BARE_SHORTCUT_CODES entry that is a real event.code (the handler compares against
    // event.code) switches a mouse-focused slider to keyboard origin, after which letters stop
    // playing on it. Enumerated from the exported list so "every" is proven, not stated.
    // Space is excluded by the outer guard; '+', '=', '-', '_' are key values (the TimeGrid zoom
    // keys), never event.code values, so they can never match here — asserted dead just below.
    const KEY_ONLY = new Set(['+', '=', '-', '_'])
    const codes = BARE_SHORTCUT_CODES.filter((c) => c !== 'Space' && !KEY_ONLY.has(c))
    expect(codes).toEqual([
      'F2',
      'Escape',
      'Enter',
      'ArrowUp',
      'ArrowDown',
      'ArrowLeft',
      'ArrowRight',
      'Home',
      'Delete',
      'Backspace',
      'PageUp',
      'PageDown',
      'End',
      'Tab',
    ])
    for (const code of codes) {
      k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
      k.focus()
      expect(playsNote({ ...base, target: k }), `${code} before`).toBe(true) // pointer origin
      k.dispatchEvent(new KeyboardEvent('keydown', { key: code, code, bubbles: true }))
      expect(playsNote({ ...base, target: k }), `${code} after`).toBe(false) // keyboard origin: slider keeps letters
    }

    // The four key-value entries never match event.code, so they do not reassert the origin.
    for (const value of KEY_ONLY) {
      k.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
      k.focus()
      // A zoom key's real event.code is Equal/Minus/NumpadAdd/NumpadSubtract, not '+'/'='/'-'/'_'.
      k.dispatchEvent(new KeyboardEvent('keydown', { key: value, code: value === '+' || value === '=' ? 'Equal' : 'Minus', bubbles: true }))
      expect(playsNote({ ...base, target: k }), `${value} stays pointer`).toBe(true) // not reasserted
    }

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
      'lib/components/ColorPicker.svelte': ['End', 'Escape', 'Home', 'Home', 'Tab'], // the palette's grid of a popup, like a menu: it reads no letter; arrows, Home, End, Escape and Tab only
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
      'views/mixer/MixerView.svelte': ['Escape'], // closes the add-track menu
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
      '<a href="#"></a>',
      '<button role="radio" aria-checked="false"></button>',
      '<div role="radiogroup"><button></button></div>',
      '<div role="grid"><button></button></div>',
      '<div role="tablist"><button></button></div>',
      '<input type="checkbox">',
      '<input type="radio">',
      '<input type="range">',
      '<div role="slider" tabindex="0"></div>',
    ]) {
      const root = fixture(html)
      const target = root.querySelector('button') ?? root
      expect(
        playsNote(event({ target }), () => true),
        `${html} focused by the keyboard`,
      ).toBe(false)
      expect(
        playsNote(event({ target }), () => false),
        `${html} focused by the mouse`,
      ).toBe(true)
    }
    // A plain push button uses Space and Enter only: the letters play on it, however it got the focus (#472).
    for (const html of ['<button></button>', '<button role="button"></button>', '<button><span></span></button>']) {
      const target = fixture(html)
      expect(
        playsNote(event({ target: target.querySelector('span') ?? target }), () => true),
        `${html} focused by the keyboard`,
      ).toBe(true)
      expect(isPlainButton(target), html).toBe(true)
    }
    expect(isPlainButton(fixture('<a href="#"></a>'))).toBe(false)
    expect(isPlainButton(fixture('<button role="menuitem"></button>'))).toBe(false)
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

describe('layout shortcuts (#472)', () => {
  const alt = { altKey: true, ctrlKey: false, metaKey: false, shiftKey: false, repeat: false, target: null }
  it('Alt+1 to Alt+9 (digit row or numpad) pick the nth main view, Alt+M the bottom panel', () => {
    expect(layoutShortcut({ ...alt, code: 'Digit1' })).toEqual({ view: 0 })
    expect(layoutShortcut({ ...alt, code: 'Numpad3' })).toEqual({ view: 2 })
    expect(layoutShortcut({ ...alt, code: 'KeyM' })).toBe('panel')
  })
  it('needs Alt alone: no bare key, no Ctrl, Cmd or Shift, no repeat, nothing in a text field', () => {
    expect(layoutShortcut({ ...alt, altKey: false, code: 'Digit1' })).toBeNull()
    expect(layoutShortcut({ ...alt, altKey: false, code: 'KeyM' })).toBeNull()
    for (const held of ['ctrlKey', 'metaKey', 'shiftKey', 'repeat'] as const) expect(layoutShortcut({ ...alt, [held]: true, code: 'Digit2' }), held).toBeNull()
    expect(layoutShortcut({ ...alt, code: 'KeyA' })).toBeNull()
    expect(layoutShortcut({ ...alt, code: 'Digit0' })).toBeNull()
    const input = document.createElement('input')
    input.type = 'text'
    expect(layoutShortcut({ ...alt, code: 'KeyM', target: input })).toBeNull()
  })
  it('never plays a note: playing ignores every Alt event', () => {
    for (const code of ['Digit1', 'Digit2', 'Digit3', 'KeyM']) {
      expect(ignoreKeyEvent({ ...alt, code } as never), code).toBe(true)
    }
  })
})
