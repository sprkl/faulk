import { describe, expect, it } from 'vitest'
import type { Command, Track } from '../generated/protocol'
import type { BrowserItem } from './catalog'
import { applyItem, createsTrack, type DropHost } from './drop'

type Devices = Track['devices']

/** A project reduced to what a drop reads: tracks with their chains, and the commands run, each applied as the core would. */
function fake(refuse: (c: Command) => boolean = () => false) {
  const tracks: { id: string; track: Track }[] = []
  const master: Devices = {}
  const log: Command[] = []
  let selected: string | null = null
  let masterSelected = false
  let seq = 0
  const device = (kind: string, role: 'instrument' | 'effect', order: number) =>
    ({ order: String(order).padStart(4, '0'), role, enabled: true, source: { builtin: { id: kind } } }) as unknown as Devices[string]
  const insertAt = (devices: Devices, kind: string, index: number): string => {
    const effects = Object.entries(devices)
      .filter(([, d]) => d?.role === 'effect')
      .sort(([, a], [, b]) => (a!.order < b!.order ? -1 : 1))
    const id = `d${++seq}`
    // Orders are re-dealt: instrument 0, then the effects in their new order.
    const ids = effects.map(([k]) => k)
    ids.splice(index, 0, id)
    devices[id] = device(kind, 'effect', 0)
    ids.forEach((k, i) => (devices[k]!.order = String(i + 1).padStart(4, '0')))
    return id
  }
  const host: DropHost = {
    run(command) {
      log.push(command)
      if (refuse(command)) return false
      switch (command.type) {
        case 'track.create': {
          const id = `t${++seq}`
          const devices: Devices = {}
          if (command.kind === 'midi') devices[`i${++seq}`] = device(command.instrument ?? 'faulk.osc_synth', 'instrument', 0)
          tracks.push({ id, track: { kind: command.kind, devices } as unknown as Track })
          return true
        }
        case 'track.set_instrument': {
          const entry = tracks.find((t) => t.id === command.track)!
          for (const k of Object.keys(entry.track.devices)) if (entry.track.devices[k]?.role === 'instrument') delete entry.track.devices[k]
          entry.track.devices[`i${++seq}`] = device(command.instrument, 'instrument', 0)
          return true
        }
        case 'device.insert': {
          if ('master' in command && command.master) insertAt(master, command.kind, command.index)
          else insertAt(tracks.find((t) => t.id === (command as { track: string }).track)!.track.devices, command.kind, command.index)
          return true
        }
        default:
          return true
      }
    },
    tracks: () => tracks,
    masterDevices: () => master,
    selectedTrack: () => selected,
    masterSelected: () => masterSelected,
  }
  return {
    host,
    log,
    tracks,
    master,
    select: (id: string | null, isMaster = false) => {
      selected = id
      masterSelected = isMaster
    },
    types: () => log.map((c) => c.type),
  }
}

const item = (kind: string, category: BrowserItem['category'], preset: string | null = null): BrowserItem => ({
  kind,
  name: kind.replace('faulk.', ''),
  category,
  preset: preset ? { ref: { location: 'factory', id: `${kind}/${preset}` }, name: preset, category: null } : null,
})
const synth = item('faulk.osc_synth', 'instrument')
const drums = item('faulk.drums', 'instrument')
const reverb = item('faulk.reverb', 'effect')
const echo = item('faulk.echo', 'effect')

const kinds = (devices: Devices) =>
  Object.values(devices)
    .sort((a, b) => (a!.order < b!.order ? -1 : 1))
    .map((d) => ('builtin' in d!.source ? d!.source.builtin.id : '?'))

describe('what a drop does', () => {
  it('an instrument on the empty area: track.create, one command, no group', () => {
    const f = fake()
    const result = applyItem(f.host, drums, { type: 'new-track' })
    expect(result.ok).toBe(true)
    expect(f.log).toEqual([{ type: 'track.create', kind: 'midi', instrument: 'faulk.drums', color: undefined }])
    expect(result.track).toBe(f.tracks[0]!.id)
    expect(result.said.key).toBe('browser.said.created')
  })

  it('a preset of an instrument on the empty area: track.create then preset.load, as one history group', () => {
    const f = fake()
    const result = applyItem(f.host, item('faulk.osc_synth', 'instrument', 'Round Bass'), { type: 'new-track' })
    expect(result.ok).toBe(true)
    expect(f.types()).toEqual(['history.begin_group', 'track.create', 'preset.load', 'history.end_group'])
    const load = f.log[2] as Extract<Command, { type: 'preset.load' }>
    expect(load.preset).toEqual({ location: 'factory', id: 'faulk.osc_synth/Round Bass' })
    const instrument = Object.keys(f.tracks[0]!.track.devices)[0]
    expect(load.device).toBe(instrument)
  })

  it('an effect on a track lands at the dropped index, among the effects', () => {
    const f = fake()
    applyItem(f.host, synth, { type: 'new-track' })
    const track = f.tracks[0]!.id
    f.log.length = 0
    applyItem(f.host, reverb, { type: 'track', track })
    applyItem(f.host, echo, { type: 'track', track, index: 0 })
    expect(f.log).toEqual([
      { type: 'device.insert', track, kind: 'faulk.reverb', index: 0 },
      { type: 'device.insert', track, kind: 'faulk.echo', index: 0 },
    ])
    expect(kinds(f.tracks[0]!.track.devices)).toEqual(['faulk.osc_synth', 'faulk.echo', 'faulk.reverb'])
  })

  it('an index past the chain is the end of it, a negative one the start', () => {
    const f = fake()
    applyItem(f.host, synth, { type: 'new-track' })
    const track = f.tracks[0]!.id
    applyItem(f.host, reverb, { type: 'track', track })
    f.log.length = 0
    applyItem(f.host, echo, { type: 'track', track, index: 99 })
    applyItem(f.host, item('faulk.filter', 'effect'), { type: 'track', track, index: -3 })
    expect(f.log.map((c) => (c as { index: number }).index)).toEqual([1, 0])
  })

  it('a preset of an effect on a track: device.insert at the place, then preset.load on the device it made, in one group', () => {
    const f = fake()
    applyItem(f.host, synth, { type: 'new-track' })
    const track = f.tracks[0]!.id
    applyItem(f.host, reverb, { type: 'track', track })
    f.log.length = 0
    applyItem(f.host, item('faulk.echo', 'effect', 'Slapback'), { type: 'track', track, index: 0 })
    expect(f.types()).toEqual(['history.begin_group', 'device.insert', 'preset.load', 'history.end_group'])
    const load = f.log[2] as Extract<Command, { type: 'preset.load' }>
    const made = Object.entries(f.tracks[0]!.track.devices).find(([, d]) => 'builtin' in d!.source && d!.source.builtin.id === 'faulk.echo')![0]
    expect(load.device).toBe(made)
  })

  it('an instrument on a MIDI track replaces its instrument; the same instrument changes nothing, its preset loads', () => {
    const f = fake()
    applyItem(f.host, synth, { type: 'new-track' })
    const track = f.tracks[0]!.id
    f.log.length = 0
    expect(applyItem(f.host, drums, { type: 'track', track }).ok).toBe(true)
    expect(f.log).toEqual([{ type: 'track.set_instrument', track, instrument: 'faulk.drums' }])
    f.log.length = 0
    applyItem(f.host, item('faulk.drums', 'instrument', 'Deep Boom'), { type: 'track', track })
    expect(f.types()).toEqual(['preset.load'])
    f.log.length = 0
    applyItem(f.host, item('faulk.osc_synth', 'instrument', 'Soft Pad'), { type: 'track', track })
    expect(f.types()).toEqual(['history.begin_group', 'track.set_instrument', 'preset.load', 'history.end_group'])
  })

  it('an effect on the master goes through device.insert with master: true', () => {
    const f = fake()
    applyItem(f.host, reverb, { type: 'master' })
    applyItem(f.host, echo, { type: 'master', index: 0 })
    expect(f.log).toEqual([
      { type: 'device.insert', master: true, kind: 'faulk.reverb', index: 0 },
      { type: 'device.insert', master: true, kind: 'faulk.echo', index: 0 },
    ])
  })

  it('an instrument cannot go to the master: nothing runs and it says why', () => {
    const f = fake()
    const result = applyItem(f.host, synth, { type: 'master' })
    expect(result.ok).toBe(false)
    expect(result.said.key).toBe('browser.said.noInstrumentOnMaster')
    expect(f.log).toEqual([])
  })

  it('a track that is not there is refused without a command', () => {
    const f = fake()
    expect(applyItem(f.host, reverb, { type: 'track', track: 'gone' }).ok).toBe(false)
    expect(f.log).toEqual([])
  })

  it('an instrument on an audio track starts a MIDI track instead', () => {
    const f = fake()
    f.host.run({ type: 'track.create', kind: 'audio' } as Command)
    f.log.length = 0
    const result = applyItem(f.host, synth, { type: 'track', track: f.tracks[0]!.id })
    expect(result.ok).toBe(true)
    expect(f.tracks).toHaveLength(2)
    expect(f.log[0]!.type).toBe('track.create')
  })
})

describe('the keyboard: Enter or a double-click', () => {
  it('goes to the selected track: an effect at the end of its chain, an instrument as its instrument', () => {
    const f = fake()
    applyItem(f.host, synth, { type: 'new-track' })
    const track = f.tracks[0]!.id
    f.select(track)
    f.log.length = 0
    applyItem(f.host, reverb, { type: 'selection' })
    applyItem(f.host, echo, { type: 'selection' })
    applyItem(f.host, drums, { type: 'selection' })
    expect(f.log).toEqual([
      { type: 'device.insert', track, kind: 'faulk.reverb', index: 0 },
      { type: 'device.insert', track, kind: 'faulk.echo', index: 1 },
      { type: 'track.set_instrument', track, instrument: 'faulk.drums' },
    ])
  })

  it('creates the track when there is none selected: an instrument with it, an effect with the first instrument', () => {
    const a = fake()
    applyItem(a.host, drums, { type: 'selection' })
    expect(a.log[0]).toMatchObject({ type: 'track.create', kind: 'midi', instrument: 'faulk.drums' })
    const b = fake()
    const result = applyItem(b.host, reverb, { type: 'selection' })
    expect(result.ok).toBe(true)
    expect(b.types()).toEqual(['history.begin_group', 'track.create', 'device.insert', 'history.end_group'])
    expect(b.log[1]).toMatchObject({ kind: 'midi', instrument: 'faulk.osc_synth' })
    expect(kinds(b.tracks[0]!.track.devices)).toEqual(['faulk.osc_synth', 'faulk.reverb'])
  })

  it('goes to the master when the master is selected and the item is an effect', () => {
    const f = fake()
    applyItem(f.host, synth, { type: 'new-track' })
    f.select(f.tracks[0]!.id, true)
    f.log.length = 0
    applyItem(f.host, reverb, { type: 'selection' })
    expect(f.log).toEqual([{ type: 'device.insert', master: true, kind: 'faulk.reverb', index: 0 }])
    f.log.length = 0
    applyItem(f.host, drums, { type: 'selection' })
    expect(f.log[0]!.type).toBe('track.create')
  })
})

describe('a refused step', () => {
  it('still closes the history group, and stops the sequence', () => {
    const f = fake((c) => c.type === 'preset.load')
    const result = applyItem(f.host, item('faulk.osc_synth', 'instrument', 'Round Bass'), { type: 'new-track' })
    expect(result.ok).toBe(false)
    expect(f.types()).toEqual(['history.begin_group', 'track.create', 'preset.load', 'history.end_group'])
  })

  it('a refused track.create stops before the preset, and says nothing was created', () => {
    const f = fake((c) => c.type === 'track.create')
    const result = applyItem(f.host, item('faulk.osc_synth', 'instrument', 'Round Bass'), { type: 'new-track' })
    expect(result.ok).toBe(false)
    expect(result.track).toBeNull()
    expect(f.types()).toEqual(['history.begin_group', 'track.create', 'history.end_group'])
  })
})

describe('a group already open', () => {
  it('a refused begin_group: the steps still run in that group, and no end_group is sent', () => {
    const f = fake((c) => c.type === 'history.begin_group')
    const result = applyItem(f.host, item('faulk.osc_synth', 'instrument', 'Round Bass'), { type: 'new-track' })
    expect(result.ok).toBe(true)
    expect(f.types()).toEqual(['history.begin_group', 'track.create', 'preset.load'])
  })

  it('a refused begin_group and a refused step: still no end_group', () => {
    const f = fake((c) => c.type === 'history.begin_group' || c.type === 'preset.load')
    const result = applyItem(f.host, item('faulk.osc_synth', 'instrument', 'Round Bass'), { type: 'new-track' })
    expect(result.ok).toBe(false)
    expect(f.types()).not.toContain('history.end_group')
  })
})

describe('what makes a track', () => {
  it('only an instrument or a preset of one', () => {
    expect(createsTrack(synth)).toBe(true)
    expect(createsTrack(item('faulk.osc_synth', 'instrument', 'Round Bass'))).toBe(true)
    expect(createsTrack(reverb)).toBe(false)
    expect(createsTrack(item('faulk.x', 'midi_effect'))).toBe(false)
  })
})
