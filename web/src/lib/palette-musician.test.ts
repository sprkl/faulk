import { describe, expect, it } from 'vitest'
import { listFor, paletteEntries, parseQuery, SUMMARY_PREFIX, PALETTE_LABEL_PREFIX } from './command-palette'
import { COMMAND_CATALOG } from './generated/protocol'
import { messages } from './i18n/app'
import { translate, type MessageKey } from './i18n'
import { GESTURES, GESTURE_IDS, runGesture, type GestureUi } from './palette-gestures'
import { HIDDEN_COMMANDS, HIDDEN_GROUPS } from './palette-hidden'
import { commandShortcutText, labelShortcutText, SHORTCUT_GROUPS } from './shortcuts'
import { SHORTCUT_COMMANDS } from './shortcut-commands'
import { COMMAND_LABEL_PREFIX } from './i18n/history'
import { UiStore, GESTURE_TTL_MS } from './stores/ui.svelte'

const langs = ['en', 'fr'] as const
const text = (lang: 'en' | 'fr') => (key: MessageKey) => translate(lang, key)
const names = new Set(COMMAND_CATALOG.map((c) => c.name))
const visible = COMMAND_CATALOG.filter((c) => !HIDDEN_COMMANDS.has(c.name))
const hasKey = (lang: 'en' | 'fr', key: string) => key in messages[lang]

describe('the hidden commands (#926)', () => {
  it('names only commands of the catalogue, each in one group', () => {
    const all = Object.values(HIDDEN_GROUPS).flat()
    for (const name of all) expect(names.has(name), name).toBe(true)
    expect(new Set(all).size).toBe(all.length)
    expect(HIDDEN_COMMANDS.size).toBe(all.length)
  })

  it('hides every inverse of the history, every host-driven command and every grouping of the history, and no other history command', () => {
    for (const c of COMMAND_CATALOG) {
      if (/(^|\.)restore(_|$)/.test(c.name) || /^record\.restore$/.test(c.name)) expect(HIDDEN_COMMANDS.has(c.name), c.name).toBe(true)
      if (c.name.startsWith('history.') && !['history.undo', 'history.redo'].includes(c.name)) expect(HIDDEN_COMMANDS.has(c.name), c.name).toBe(true)
    }
    for (const name of ['project.open', 'project.keep_mine', 'project.load_folder_version']) expect(HIDDEN_COMMANDS.has(name), name).toBe(true)
    for (const name of ['history.undo', 'history.redo']) expect(HIDDEN_COMMANDS.has(name), name).toBe(false)
  })
})

describe('what each listed command says (#926)', () => {
  for (const lang of langs) {
    it(`has a title and a one-line summary in ${lang} for every command listed by default, with no parameter name and no backtick`, () => {
      expect(visible.length).toBeGreaterThan(80)
      for (const c of visible) {
        const title = [PALETTE_LABEL_PREFIX, COMMAND_LABEL_PREFIX].map((p) => p + c.name).find((k) => hasKey(lang, k))
        expect(title, `${lang} title of ${c.name}`).toBeDefined()
        const summaryKey = SUMMARY_PREFIX + c.name
        expect(hasKey(lang, summaryKey), `${lang} summary of ${c.name}`).toBe(true)
        const summary = messages[lang][summaryKey as MessageKey]
        const label = messages[lang][title as MessageKey]
        for (const value of [summary, label]) {
          expect(value, c.name).not.toContain('`')
          expect(value, c.name).not.toContain('\n')
          expect(value.trim(), c.name).not.toBe('')
          expect(value, c.name).not.toMatch(/\b[a-z]+_[a-z_]+\b/)
        }
        expect(summary.length, c.name).toBeLessThanOrEqual(120)
      }
    })

    it(`has a title, a summary and keys for every gesture in ${lang}`, () => {
      for (const g of GESTURES) {
        expect(hasKey(lang, `palette.gesture.${g.id}`), g.id).toBe(true)
        expect(hasKey(lang, `palette.gsum.${g.id}`), g.id).toBe(true)
        expect(messages[lang][`palette.gsum.${g.id}` as MessageKey], g.id).not.toContain('`')
      }
    })
  }

  it('keeps no summary or title for a command that does not exist', () => {
    for (const lang of langs)
      for (const key of Object.keys(messages[lang]).filter((k) => k.startsWith(SUMMARY_PREFIX)))
        expect(names.has(key.slice(SUMMARY_PREFIX.length)), key).toBe(true)
  })
})

describe('the two levels of the palette (#926)', () => {
  const entries = paletteEntries(text('en'))
  const list = (query: string) => listFor(entries, query).map((e) => e.entry.name)

  it('reads a > first as the whole catalogue, after spaces too, and any other > as a character', () => {
    expect(parseQuery('>tempo')).toEqual({ all: true, text: 'tempo' })
    expect(parseQuery('  > tempo')).toEqual({ all: true, text: ' tempo' })
    expect(parseQuery('>')).toEqual({ all: true, text: '' })
    expect(parseQuery('a > b')).toEqual({ all: false, text: 'a > b' })
  })

  it('lists no hidden command by default, not even found by its agent name, and lists them after >', () => {
    for (const name of HIDDEN_COMMANDS) {
      expect(list(''), name).not.toContain(name)
      expect(list(name), name).not.toContain(name)
      expect(list('>' + name), name).toContain(name)
    }
    expect(list('>').length).toBe(COMMAND_CATALOG.length + GESTURES.length)
    expect(list('').length).toBe(visible.length + GESTURES.length)
  })

  it('finds a command by its agent name without showing it', () => {
    expect(list('clip.notes.transpose')).toContain('clip.notes.transpose')
  })

  it('finds a hidden command only by its own words after >', () => {
    expect(list('restaurer')).toEqual([])
    expect(list('track restore')).not.toContain('track.restore')
    expect(list('>track restore')).toContain('track.restore')
  })
})

describe('the gestures of the registry (#926)', () => {
  const labels = SHORTCUT_GROUPS.flatMap((g) => g.shortcuts.map((s) => s.label))
  const commandLabels = new Set(Object.values(SHORTCUT_COMMANDS))
  const gestureLabels = new Set(GESTURES.map((g) => g.shortcut))

  /**
   * The lines of the registry that are neither a gesture nor a command of the palette, with why. A key that needs a focus
   * (an item, a cursor, a lane, a point, a note, a slider) acts on what holds it, which the palette cannot give; the rest is
   * a key the page reads itself.
   */
  const NOT_IN_PALETTE: Readonly<Record<string, 'focus' | 'keys' | 'later'>> = {
    'shortcut.renameProject': 'focus',
    'shortcut.views': 'keys',
    'shortcut.tap': 'later',
    'shortcut.palette': 'keys',
    'shortcut.tab': 'keys',
    'shortcut.cancel': 'keys',
    'shortcut.typing': 'keys',
    'shortcut.notes': 'keys',
    'shortcut.octave': 'keys',
    'shortcut.tlScroll': 'focus',
    'shortcut.tlStart': 'focus',
    'shortcut.tlZoom': 'focus',
    'shortcut.markerJump': 'later',
    'shortcut.clipMove': 'focus',
    'shortcut.clipExtend': 'focus',
    'shortcut.clipShift': 'focus',
    'shortcut.clipResize': 'focus',
    'shortcut.clipOpen': 'focus',
    'shortcut.clipDuplicate': 'focus',
    'shortcut.clipDuplicateLinked': 'focus',
    'shortcut.cursorFree': 'focus',
    'shortcut.cursorMove': 'focus',
    'shortcut.cursorBar': 'focus',
    'shortcut.clipCreate': 'focus',
    'shortcut.playheadCursor': 'focus',
    'shortcut.markerMove': 'focus',
    'shortcut.remove': 'focus',
    'shortcut.rename': 'focus',
    'shortcut.laneVolume': 'focus',
    'shortcut.laneHeight': 'focus',
    'shortcut.lanePoint': 'focus',
    'shortcut.lanePoints': 'focus',
    'shortcut.pointStep': 'focus',
    'shortcut.pointEdit': 'focus',
    'shortcut.pointNudge': 'focus',
    'shortcut.pointRemove': 'focus',
    'shortcut.pointCurve': 'focus',
    'shortcut.noteAll': 'focus',
    'shortcut.noteCopy': 'focus',
    'shortcut.noteDuplicate': 'focus',
    'shortcut.noteRepeat': 'later',
    'shortcut.noteGenerate': 'later',
    'shortcut.noteQuantize': 'later',
    'shortcut.stepRest': 'focus',
    'shortcut.stepLengthen': 'focus',
    'shortcut.stepBack': 'focus',
    'shortcut.noteStep': 'focus',
    'shortcut.noteSelect': 'focus',
    'shortcut.noteShift': 'focus',
    'shortcut.noteResize': 'focus',
    'shortcut.noteTranspose': 'later',
    'shortcut.noteCursor': 'focus',
    'shortcut.noteCreate': 'focus',
    'shortcut.noteCursorEnds': 'focus',
    'shortcut.noteCursorBar': 'focus',
    'shortcut.noteVelocity': 'focus',
    'shortcut.gridMove': 'focus',
    'shortcut.gridCopy': 'focus',
    'shortcut.sliderStep': 'focus',
    'shortcut.sliderFine': 'focus',
    'shortcut.sliderBig': 'focus',
    'shortcut.sliderDefault': 'focus',
    'shortcut.sliderType': 'focus',
    'shortcut.tempo': 'focus',
  }

  it('accounts for every line of the registry: a gesture, a command with its keys, or a written exception', () => {
    for (const label of labels) {
      const known = gestureLabels.has(label) || commandLabels.has(label) || label in NOT_IN_PALETTE
      expect(known, label).toBe(true)
    }
  })

  it('keeps no exception for a line the registry no longer has, nor for one that became a gesture', () => {
    for (const label of Object.keys(NOT_IN_PALETTE)) {
      expect(labels, label).toContain(label as MessageKey)
      expect(gestureLabels.has(label as MessageKey) || commandLabels.has(label as MessageKey), label).toBe(false)
    }
  })

  it('names, for each gesture, a line of the registry, once, and shows its keys', () => {
    expect(GESTURES.map((g) => g.id)).toEqual([...GESTURE_IDS])
    expect(new Set(GESTURES.map((g) => g.shortcut)).size).toBe(GESTURES.length)
    for (const g of GESTURES) {
      expect(labels, g.id).toContain(g.shortcut)
      expect(labelShortcutText(g.shortcut, false), g.id).not.toBe('')
    }
    expect(labelShortcutText('shortcut.fitSong', false)).toBe('Alt+Z')
    expect(labelShortcutText('shortcut.goTo', true)).toBe('Cmd+G')
  })

  it('shows Space on Play and a key on every entry whose line has one', () => {
    expect(commandShortcutText('transport.play', false)).toBe('Space')
    const entries = paletteEntries(text('en'))
    for (const e of entries.filter((x) => x.gesture)) expect(labelShortcutText(e.shortcut!, false), e.entry.name).not.toBe('')
  })

  it('finds the gestures by their names, in French and in English', () => {
    const fr = paletteEntries(text('fr'))
    const en = paletteEntries(text('en'))
    const find = (all: typeof fr, q: string) => listFor(all, q, () => '').map((e) => e.gesture)
    expect(find(fr, 'aller à')).toContain('goTo')
    expect(find(fr, 'allonger')).toContain('clipLengthen')
    expect(find(fr, 'répéter')).toContain('clipRepeat')
    expect(find(fr, 'nouveau clip')).toContain('clipNew')
    expect(find(fr, 'rampe')).toContain('laneRamp')
    expect(find(fr, 'tout le morceau')).toContain('fitSong')
    expect(find(fr, 'saisie pas à pas')).toContain('stepEntry')
    expect(find(fr, 'placement du clip')).toContain('clipPlacement')
    expect(find(en, 'clip’s placement')).toContain('clipPlacement')
    expect(find(en, 'lengthen')).toContain('clipLengthen')
    expect(find(en, 'step entry')).toContain('stepEntry')
  })

  it('finds an entry by its keys', () => {
    const entries = paletteEntries(text('en'))
    const keys = (e: (typeof entries)[number]) => (e.shortcut ? labelShortcutText(e.shortcut, false) : commandShortcutText(e.entry.name, false))
    expect(listFor(entries, 'ctrl', keys).map((e) => e.entry.name)).toContain('history.undo')
    expect(listFor(entries, 'space', keys).map((e) => e.entry.name)).toContain('transport.play')
  })
})

describe('running a gesture', () => {
  function double() {
    const calls: string[] = []
    const ui: GestureUi = {
      gotoOpen: false,
      helpOpen: false,
      showBrowser: () => calls.push('browser'),
      toggleMaximized: () => calls.push('panel'),
      show: (view) => calls.push(`show:${view}`),
      showPanel: (panel) => calls.push(`panel:${panel}`),
      requestGesture: (id) => calls.push(`ask:${id}`),
    }
    return { ui, calls }
  }

  it('opens the dialogs and panels of the shell itself', () => {
    const { ui, calls } = double()
    runGesture('goTo', ui)
    expect(ui.gotoOpen).toBe(true)
    runGesture('help', ui)
    expect(ui.helpOpen).toBe(true)
    runGesture('browser', ui)
    runGesture('panel', ui)
    expect(calls).toEqual(['browser', 'panel'])
  })

  it('shows the view that owns the others and asks it', () => {
    const { ui, calls } = double()
    runGesture('clipLengthen', ui)
    runGesture('stepEntry', ui)
    runGesture('clipPlacement', ui)
    expect(calls).toEqual(['show:arrangement', 'ask:clipLengthen', 'panel:piano-roll', 'ask:stepEntry', 'panel:piano-roll', 'ask:clipPlacement'])
  })
})

describe('the request of a gesture', () => {
  it('is taken once by its owner, left to another owner, and dropped when nobody took it in time', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.requestGesture('fitSong')
    expect(ui.takeGesture(['stepEntry'])).toBeNull()
    expect(ui.gesture).not.toBeNull()
    expect(ui.takeGesture(['fitSong'])).toBe('fitSong')
    expect(ui.takeGesture(['fitSong'])).toBeNull()
    ui.requestGesture('fitSong')
    expect(ui.takeGesture(['fitSong'], Date.now() + GESTURE_TTL_MS + 1)).toBeNull()
    expect(ui.gesture).toBeNull()
  })

  it('keeps only the last request', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.requestGesture('fitSong')
    ui.requestGesture('clipNew')
    expect(ui.takeGesture(['fitSong'])).toBeNull()
    expect(ui.takeGesture(['clipNew'])).toBe('clipNew')
  })
})
