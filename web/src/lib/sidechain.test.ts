import { describe, expect, it } from 'vitest'
import { DEVICES, type CommandError, type Track } from './generated/protocol'
import { deletedTrack, lostNotice, setSidechain, sidechainRefusal, sourceOptions, sourceUnheard, trackRef } from './sidechain'
import { messages } from './i18n/app'

const mixer = { mute: false, pan: 0, gain_db: 0 }
function track(order: string, over: Partial<Track> = {}): Track {
  return { automation: {}, clips: {}, devices: {}, kind: 'midi', mixer, name: '', order, output: 'master', placements: {}, ...over }
}
const t = (_key: 'tracks.defaultName', vars: { n: number }) => `Track ${vars.n}`

const project = {
  tracks: {
    pad: track('a', { name: 'Pad' }),
    kick: track('b', { name: 'Kick' }),
    kick2: track('c', { name: 'Kick' }),
    blank: track('d'),
  },
}

describe('the detection source of a device (#952)', () => {
  it('is set by one command, null for the own track', () => {
    expect(setSidechain('d', 'kick')).toEqual({ type: 'device.set_sidechain', device: 'd', source: 'kick' })
    expect(setSidechain('d', null)).toEqual({ type: 'device.set_sidechain', device: 'd', source: null })
  })

  it('offers every other track by its shown name, with its position when two are shown alike', () => {
    expect(sourceOptions(project, 'pad', t)).toEqual([
      { id: 'kick', label: 'Kick (2)' },
      { id: 'kick2', label: 'Kick (3)' },
      { id: 'blank', label: 'Track 4' },
    ])
    expect(sourceOptions({ tracks: { pad: track('a') } }, 'pad', t)).toEqual([])
  })

  it('names the track of a loop the core refused, by name or by position', () => {
    const loop: CommandError = { code: 'invalid', field: 'source', message: '"Kick" already listens to this track' }
    expect(sidechainRefusal(setSidechain('d', 'kick'), loop, project)).toEqual({ key: 'refusal.sidechainCycle', vars: { track: 'Kick' } })
    expect(sidechainRefusal(setSidechain('d', 'blank'), loop, project)).toEqual({ key: 'refusal.sidechainCycleUnnamed', vars: { n: 4 } })
    // Any other refusal, or another command, keeps its generic text.
    const gone: CommandError = { code: 'not_found', entity: 'track', id: 'x' }
    expect(sidechainRefusal(setSidechain('d', 'kick'), gone, project)).toBeNull()
    expect(sidechainRefusal(setSidechain('d', null), loop, project)).toBeNull()
    expect(sidechainRefusal({ type: 'device.set_bypass', device: 'd', bypass: true }, loop, project)).toBeNull()
    expect(sidechainRefusal(setSidechain('d', 'nowhere'), loop, project)).toBeNull()
  })

  it('tells that the devices of a deleted source listen to their own track, only when there were some', () => {
    const del = (track: string) => deletedTrack({ type: 'track.delete', track }, project)
    const out = (sidechains: string[]) => ({ type: 'track.delete', devices: 1, clips: 0, sidechains })
    expect(lostNotice(del('kick'), out(['c1']))).toEqual({ key: 'devices.sidechainLost', vars: { track: 'Kick', count: 1 } })
    expect(lostNotice(del('blank'), out(['c1', 'c2']))).toEqual({ key: 'devices.sidechainLostUnnamed', vars: { n: 4, count: 2 } })
    expect(lostNotice(del('kick'), out([]))).toBeNull()
    expect(lostNotice(del('kick'), null)).toBeNull()
    expect(lostNotice(null, out(['c1']))).toBeNull()
    expect(deletedTrack({ type: 'track.rename', track: 'kick', name: 'K' }, project)).toBeNull()
    expect(trackRef(undefined, 'kick')).toBeNull()
  })

  it('knows when the source is not heard: muted, or another track soloed', () => {
    const muted = { tracks: { ...project.tracks, kick: track('b', { name: 'Kick', mixer: { ...mixer, mute: true } }) } }
    expect(sourceUnheard(project, [], 'kick')).toBe(false)
    expect(sourceUnheard(muted, [], 'kick')).toBe(true)
    expect(sourceUnheard(project, ['pad'], 'kick')).toBe(true)
    expect(sourceUnheard(project, ['kick'], 'kick')).toBe(false)
    expect(sourceUnheard(project, [], 'gone')).toBe(false)
  })

  it('is offered for the Compressor only, and its words exist in both languages', () => {
    expect(DEVICES.filter((d) => d.sidechain).map((d) => d.kind)).toEqual(['faulk.compressor'])
    for (const key of [
      'devices.sidechain',
      'devices.sidechainOwn',
      'devices.sidechainHint',
      'devices.sidechainUnheard',
      'devices.sidechainLost',
      'devices.sidechainLostUnnamed',
      'refusal.sidechainCycle',
      'refusal.sidechainCycleUnnamed',
      'refusal.entity.sidechainSources',
    ] as const) {
      expect(messages.en[key], key).toBeTruthy()
      expect(messages.fr[key], key).toBeTruthy()
    }
  })
})
