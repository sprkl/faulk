import { describe, expect, it } from 'vitest'
import { DRUMS_OCTAVE } from '../keyboard'
import type { Command } from '../generated/protocol'
import { KeyboardPlayer } from './keyboard-player.svelte'
import { UiStore } from './ui.svelte'

const midi = (builtin: string) => ({ kind: 'midi', devices: { d: { role: 'instrument', source: { builtin: { id: builtin } } } } })
const entry = (id: string, track: unknown) => ({ id, index: 0, track })

function setup(over: { selected?: ReturnType<typeof entry> | null; armed?: string[]; started?: boolean } = {}) {
  const sent: Command[] = []
  const selected = over.selected === undefined ? entry('t1', midi('faulk.osc_synth')) : over.selected
  const project = { selected, armed: over.armed ?? [], tracks: selected ? [selected] : [] }
  const engine = { started: over.started ?? true }
  const keys = new KeyboardPlayer(project as never, engine as never, new UiStore('play', 'devices'), { run: (c: Command) => void sent.push(c) } as never)
  return { keys, sent, project }
}

const key = (code: string, over: Record<string, unknown> = {}) =>
  ({ code, repeat: false, ctrlKey: false, metaKey: false, altKey: false, defaultPrevented: false, target: document.body, ...over }) as unknown as KeyboardEvent

describe('the computer keyboard played from the shell (#361)', () => {
  it('plays the selected MIDI track whatever the view: the player knows no view', () => {
    const { keys, sent } = setup()
    keys.keyDown(key('KeyA'))
    keys.keyUp(key('KeyA'))
    expect(sent).toEqual([
      { type: 'live.input_note_on', note: 60, velocity: 100 },
      { type: 'live.input_note_off', note: 60 },
    ])
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

  it('releaseAll ends every held note once', () => {
    const { keys, sent } = setup()
    keys.keyDown(key('KeyA'))
    keys.keyDown(key('KeyS'))
    keys.releaseAll()
    keys.releaseAll()
    expect(sent.filter((c) => c.type === 'live.input_note_off')).toHaveLength(2)
  })
})
