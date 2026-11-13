import { describe, expect, it } from 'vitest'
import { DRUMS_OCTAVE } from '../keyboard'
import type { Command } from '../generated/protocol'
import { KeyboardPlayer } from './keyboard-player.svelte'
import { UiStore } from './ui.svelte'

const midi = (builtin: string) => ({ kind: 'midi', devices: { d: { role: 'instrument', source: { builtin: { id: builtin } } } } })
const entry = (id: string, track: unknown) => ({ id, index: 0, track })

function setup(over: { selected?: ReturnType<typeof entry> | null; armed?: string[]; started?: boolean } = {}) {
  const sent: Command[] = []
  const stamps: (number | undefined)[] = []
  const selected = over.selected === undefined ? entry('t1', midi('faulk.osc_synth')) : over.selected
  const project = { selected, armed: over.armed ?? [], tracks: selected ? [selected] : [] }
  const engine = { started: over.started ?? true }
  const ui = new UiStore('arrangement', 'devices')
  const keys = new KeyboardPlayer(project as never, engine as never, ui, {
    run: (c: Command) => void sent.push(c),
    runPlayed: (c: Command, timeStamp?: number) => {
      sent.push(c)
      stamps.push(timeStamp)
    },
  } as never)
  return { keys, sent, stamps, project, ui }
}

const key = (code: string, over: Record<string, unknown> = {}) =>
  ({
    code,
    timeStamp: 1234.5,
    repeat: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    defaultPrevented: false,
    target: document.body,
    ...over,
  }) as unknown as KeyboardEvent

describe('the computer keyboard played from the shell (#361)', () => {
  it('ends step entry when musical typing is switched off (#882)', () => {
    const { keys, ui } = setup()
    ui.stepEntry = true
    keys.toggleTyping()
    expect(ui.typing).toBe('shortcuts')
    expect(ui.stepEntry).toBe(false)
  })

  it('plays no note live while the piano roll writes them (step entry, #882), and Z and X still move the octave', () => {
    const { keys, sent, ui } = setup()
    ui.stepEntry = true
    keys.keyDown(key('KeyA'))
    keys.keyUp(key('KeyA'))
    expect(sent).toEqual([])
    keys.keyDown(key('KeyX'))
    expect(keys.octave).toBe(5)
    ui.stepEntry = false
    keys.keyDown(key('KeyA'))
    expect(sent).toEqual([{ type: 'live.input_note_on', note: 72, velocity: 100 }])
  })

  it('plays the selected MIDI track whatever the view: the player knows no view', () => {
    const { keys, sent, stamps } = setup()
    keys.keyDown(key('KeyA'))
    keys.keyUp(key('KeyA', { timeStamp: 1400 }))
    expect(sent).toEqual([
      { type: 'live.input_note_on', note: 60, velocity: 100 },
      { type: 'live.input_note_off', note: 60 },
    ])
    // Each with the time of its key event, for the recording (#499).
    expect(stamps).toEqual([1234.5, 1400])
  })

  it('plays nothing before the audio started, with no target, in a text field, under a view shortcut or with a modifier', () => {
    expect(setup({ started: false }).sent).toEqual([])
    const stopped = setup({ started: false })
    stopped.keys.keyDown(key('KeyA'))
    const none = setup({ selected: null })
    none.keys.keyDown(key('KeyA'))
    const field = setup()
    field.keys.keyDown(key('KeyA', { target: Object.assign(document.createElement('input'), { type: 'text' }) }))
    const taken = setup()
    taken.keys.keyDown(key('KeyA', { defaultPrevented: true }))
    const ctrl = setup()
    ctrl.keys.keyDown(key('KeyA', { ctrlKey: true }))
    for (const s of [stopped, none, field, taken, ctrl]) expect(s.sent).toEqual([])
  })

  it('an audio track is no target', () => {
    const { keys, sent } = setup({ selected: entry('a', { kind: 'audio', devices: {} }) })
    keys.keyDown(key('KeyA'))
    expect(sent).toEqual([])
  })

  it('steps the octave with Z and X, releasing the held notes, and starts a drum kit at the drum octave', () => {
    const { keys, sent } = setup()
    keys.keyDown(key('KeyA'))
    keys.keyDown(key('KeyX'))
    expect(sent.at(-1)).toEqual({ type: 'live.input_note_off', note: 60 })
    expect(keys.octave).toBe(5)
    keys.keyUp(key('KeyA'))
    expect(sent).toHaveLength(2)
    expect(setup({ selected: entry('d', midi('faulk.drums')) }).keys.octave).toBe(DRUMS_OCTAVE)
  })

  it('toggles musical typing with the key left of 1: off, the letters and Z/X play nothing; on again, they play', () => {
    const { keys, sent, ui } = setup()
    expect(ui.typing).toBe('notes')
    keys.keyDown(key('KeyA'))
    keys.keyDown(key('Backquote'))
    // The held note ends with the mode that played it.
    expect(sent.at(-1)).toEqual({ type: 'live.input_note_off', note: 60 })
    expect(ui.typing).toBe('shortcuts')
    const before = sent.length
    keys.keyDown(key('KeyS'))
    keys.keyDown(key('KeyX'))
    expect(sent).toHaveLength(before)
    expect(keys.octave).toBe(4)
    keys.keyDown(key('Backquote'))
    expect(ui.typing).toBe('notes')
    keys.keyDown(key('KeyS'))
    expect(sent.at(-1)).toEqual({ type: 'live.input_note_on', note: 62, velocity: 100 })
  })

  it('does not toggle in a text field, with a modifier, on a repeat or before the audio started (it still toggles then)', () => {
    const { keys, ui } = setup()
    keys.keyDown(key('Backquote', { target: Object.assign(document.createElement('input'), { type: 'text' }) }))
    keys.keyDown(key('Backquote', { ctrlKey: true }))
    keys.keyDown(key('Backquote', { repeat: true }))
    keys.keyDown(key('Backquote', { shiftKey: true }))
    expect(ui.typing).toBe('notes')
    const idle = setup({ started: false })
    idle.keys.keyDown(key('Backquote'))
    expect(idle.ui.typing).toBe('shortcuts')
  })

  it('releaseAll ends every held note once', () => {
    const { keys, sent } = setup()
    keys.keyDown(key('KeyA'))
    keys.keyDown(key('KeyS'))
    keys.releaseAll()
    keys.releaseAll()
    expect(sent.filter((c) => c.type === 'live.input_note_off')).toHaveLength(2)
  })
})
