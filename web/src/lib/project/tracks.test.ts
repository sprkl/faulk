import { describe, expect, it } from 'vitest'
import type { Device, Track } from '../generated/protocol'
import {
  copyName,
  formatGain,
  formatPan,
  INSTRUMENTS,
  instrumentKind,
  instrumentName,
  sortedTracks,
  TRACK_COLORS,
  trackColor,
  trackLabel,
  trackName,
  hueKey,
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

  it('names a track from a stored name and a position with the same rule as the list (#781)', () => {
    const t = (key: 'tracks.defaultName', vars: { n: number }) => `Track ${vars.n} ${key}`
    expect(trackName('', 1, t)).toBe('Track 2 tracks.defaultName')
    expect(trackName(' 	', 0, t)).toBe('Track 1 tracks.defaultName')
    expect(trackName(' Drums ', 1, t)).toBe('Drums')
    const entry = { track: track('a', { name: '' }), index: 4 }
    expect(trackName(entry.track.name, entry.index, t)).toBe(trackLabel(entry, t))
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

  it('has palette hues written #rrggbb', () => {
    for (const color of TRACK_COLORS) expect(color).toMatch(/^#[0-9a-f]{6}$/)
  })

  it('has the sixteen hues of the design system, each named', () => {
    expect(TRACK_COLORS).toHaveLength(16)
    expect(new Set(TRACK_COLORS).size).toBe(16)
    expect(TRACK_COLORS[0]).toBe('#f08098')
    expect(TRACK_COLORS[15]).toBe('#e189c5')
    for (const color of TRACK_COLORS) expect(hueKey(color)).not.toBeNull()
    expect(hueKey('#123456')).toBeNull()
  })

  it('shows the stored colour, else a hue of the palette by position (never the grey of the secondary texts)', () => {
    expect(trackColor('#123456', 3)).toBe('#123456')
    expect(trackColor(null, 2)).toBe(TRACK_COLORS[2])
    expect(trackColor(undefined, 18)).toBe(TRACK_COLORS[2])
    expect(TRACK_COLORS).not.toContain('#8f8a85')
  })
})

describe('the name of a duplicate', () => {
  it('takes the first free number from 2, swapping a trailing number', () => {
    expect(copyName('Synth', ['Synth'])).toBe('Synth 2')
    expect(copyName('Synth', ['Synth', 'Synth 2'])).toBe('Synth 3')
    expect(copyName('Synth 2', ['Synth', 'Synth 2'])).toBe('Synth 3')
    expect(copyName('Synth', ['Synth', 'Synth 3'])).toBe('Synth 2')
    expect(copyName('Piste 4', ['Piste 4'])).toBe('Piste 5')
    expect(copyName('TR 808', ['TR 808'])).toBe('TR 808 2')
    expect(copyName('TR 808', ['TR 808', 'TR 808 2'])).toBe('TR 808 3')
    expect(copyName('Juno 106', ['Juno 106'])).toBe('Juno 106 2')
    expect(copyName('Synth 2', ['Synth 2', 'Synth 3'])).toBe('Synth 4')
  })

  it('never goes past the 200 characters a name may have, and keeps a name that is only a number', () => {
    const long = 'é'.repeat(200)
    expect([...copyName(long, [long])].length).toBeLessThanOrEqual(200)
    expect(copyName('7', ['7'])).toBe('7 2')
    const first = copyName(long, [long])
    const second = copyName(long, [long, first])
    expect(second).not.toBe(first)
    expect([...second].length).toBeLessThanOrEqual(200)
  })
})

describe('the name of a duplicate of an unnamed track', () => {
  it('is free among the names once the copy has shifted the default ones', () => {
    // [Synth, unnamed, unnamed] duplicating the second: the third becomes "Track 4" under the copy.
    expect(copyName('Track 2', ['Synth', 'Track 2', 'Track 4'])).toBe('Track 3')
    expect(copyName('Track 2', ['Synth', 'Track 2', 'Track 3', 'Track 4'])).toBe('Track 5')
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
