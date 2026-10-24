import { describe, expect, it } from 'vitest'
import type { Device, Track } from '../generated/protocol'
import {
  formatGain,
  formatPan,
  INSTRUMENTS,
  instrumentKind,
  instrumentName,
  nextColor,
  sortedTracks,
  TRACK_COLORS,
  trackLabel,
} from './tracks'

const mixer = { mute: false, pan: 0, gain_db: 0 }
function track(order: string, over: Partial<Track> = {}): Track {
  return { automation: {}, clips: {}, devices: {}, kind: 'midi', mixer, name: '', order, output: 'master', placements: {}, ...over }
}
const instrument = (id: string): Device => ({
  enabled: true,
  order: 'a',
  params: {},
  role: 'instrument',
  source: { builtin: { id, version: 1 } },
})

describe('the track list', () => {
  it('sorts by order key, then by id, and numbers the positions', () => {
    const list = sortedTracks({ tracks: { c: track('b'), a: track('b'), b: track('a') } })
    expect(list.map((e) => e.id)).toEqual(['b', 'a', 'c'])
    expect(list.map((e) => e.index)).toEqual([0, 1, 2])
  })

  it('skips an absent track and handles an empty project', () => {
    expect(sortedTracks({ tracks: {} })).toEqual([])
    expect(sortedTracks({ tracks: { a: undefined, b: track('a') } }).map((e) => e.id)).toEqual(['b'])
  })

  it('names a track without a name by its position, in the language given', () => {
    const t = (key: 'tracks.defaultName', vars: { n: number }) => `Piste ${vars.n} ${key}`
    expect(trackLabel({ track: track('a'), index: 2 }, t)).toBe('Piste 3 tracks.defaultName')
    expect(trackLabel({ track: track('a', { name: '  ' }), index: 0 }, t)).toBe('Piste 1 tracks.defaultName')
    expect(trackLabel({ track: track('a', { name: 'Bass' }), index: 2 }, t)).toBe('Bass')
  })

  it('reads the instrument of a track from its devices, by the descriptors', () => {
    const synth = track('a', { devices: { d: instrument('faulk.osc_synth') } })
    expect(instrumentKind(synth)).toBe('faulk.osc_synth')
    expect(instrumentName(synth)).toBe('Oscillator')
    expect(instrumentName(track('a', { devices: { d: instrument('faulk.drums') } }))).toBe('Drums')
    expect(instrumentName(track('a', { kind: 'audio', devices: { d: { ...instrument('faulk.clip_player'), role: 'effect' } } }))).toBeNull()
    expect(instrumentName(track('a'))).toBeNull()
  })

  it('offers the instruments of the descriptors, Oscillator and Drums', () => {
    expect(INSTRUMENTS.map((i) => i.name)).toEqual(expect.arrayContaining(['Oscillator', 'Drums']))
  })

  it('gives each new track the next colour of the palette, in a loop', () => {
    expect(nextColor(0)).toBe(TRACK_COLORS[0])
    expect(nextColor(TRACK_COLORS.length)).toBe(TRACK_COLORS[0])
    expect(nextColor(TRACK_COLORS.length + 1)).toBe(TRACK_COLORS[1])
    for (const color of TRACK_COLORS) expect(color).toMatch(/^#[0-9a-f]{6}$/)
  })
})

describe('the gain and the pan as text', () => {
  it('writes the gain and the pan for the screen', () => {
    expect(formatGain(null)).toBe('−∞')
    expect(formatGain(0)).toBe('0.0')
    expect(formatGain(-0.04)).toBe('0.0')
    expect(formatGain(-12)).toBe('−12.0')
    expect(formatGain(3.25)).toBe('3.3')
    expect(formatPan(0)).toBe('C')
    expect(formatPan(-0.3)).toBe('L30')
    expect(formatPan(1)).toBe('R100')
  })
})
