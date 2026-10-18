import { describe, expect, it } from 'vitest'
import type { Command, MidiInput } from '../generated/protocol'
import { type MidiAccessLike, type MidiInputLike, WebMidiHost } from './web-midi'
import { allNotesOff, noteOff, noteOn } from '../synth-commands'

class FakeInput implements MidiInputLike {
  state: 'connected' | 'disconnected' = 'connected'
  onmidimessage: MidiInputLike['onmidimessage'] = null
  closed = false
  constructor(
    readonly id: string,
    readonly name: string,
  ) {}
  async close() {
    this.closed = true
  }
  play(...bytes: number[]) {
    this.onmidimessage?.({ data: new Uint8Array(bytes) })
  }
}

function setup() {
  const map = new Map<string, FakeInput>([
    ['a', new FakeInput('a', 'Keys A')],
    ['b', new FakeInput('b', 'Pads B')],
  ])
  const access: MidiAccessLike = {
    inputs: { forEach: (cb) => map.forEach((v) => cb(v)), get: (id) => map.get(id) },
    onstatechange: null,
  }
  const log = { inputs: [] as [MidiInput[], string | null][], commands: [] as Command[], activity: [] as number[][] }
  const host = new WebMidiHost({
    onInputs: (inputs, selected) => log.inputs.push([inputs, selected]),
    onCommand: (c) => log.commands.push(c),
    onActivity: (n, v) => log.activity.push([n, v]),
  })
  return { map, access, log, host }
}

describe('WebMidiHost', () => {
  it('lists inputs and plays the selected one', async () => {
    const { map, access, log, host } = setup()
    expect(await host.enable(async () => access)).toBe(true)
    expect(log.inputs.at(-1)).toEqual([[{ id: 'a', name: 'Keys A' }, { id: 'b', name: 'Pads B' }], null])
    host.select('a')
    expect(log.inputs.at(-1)?.[1]).toBe('a')
    map.get('a')!.play(0x90, 60, 100)
    map.get('b')!.play(0x90, 61, 100) // not selected: ignored
    map.get('a')!.play(0x80, 60, 0)
    expect(log.commands).toEqual([
      noteOn(60, 100),
      noteOff(60),
    ])
    expect(log.activity).toEqual([
      [60, 100],
      [60, 0],
    ])
  })

  it('switching input releases the notes of the previous one', async () => {
    const { access, log, host } = setup()
    await host.enable(async () => access)
    host.select('a')
    host.select('b')
    expect(log.commands).toEqual([allNotesOff()])
  })

  it('follows hot-plug and drops an unplugged selected input', async () => {
    const { map, access, log, host } = setup()
    await host.enable(async () => access)
    host.select('a')
    map.get('a')!.state = 'disconnected'
    access.onstatechange?.({})
    expect(log.inputs.at(-1)).toEqual([[{ id: 'b', name: 'Pads B' }], null])
    expect(log.commands.at(-1)).toEqual(allNotesOff())
    map.set('c', new FakeInput('c', 'New'))
    access.onstatechange?.({})
    expect(log.inputs.at(-1)?.[0].map((i) => i.id)).toEqual(['b', 'c'])
  })

  it('releases every input before Boost mode', async () => {
    const { map, access, host } = setup()
    await host.enable(async () => access)
    host.select('a')
    await host.releaseAll()
    expect([...map.values()].every((i) => i.closed)).toBe(true)
    expect(map.get('a')!.onmidimessage).toBeNull()
  })

  it('reports refused access', async () => {
    const { host } = setup()
    expect(await host.enable(async () => Promise.reject(new Error('denied')))).toBe(false)
  })
})
