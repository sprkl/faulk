import { describe, expect, it } from 'vitest'
import { SCALE_INTERVALS } from './generated/protocol'
import { messages, translate, type MessageKey } from './i18n/app'
import { kindOf, pitchClasses, ROOTS, SCALE_MODES, scaleName, scaleShort, usesFlats } from './scale'

const en = (key: MessageKey, vars?: Record<string, string | number>) => translate('en', key, vars)
const fr = (key: MessageKey, vars?: Record<string, string | number>) => translate('fr', key, vars)

describe('the project scale on the page (#695)', () => {
  it('lists the twelve scales of the Rust table, in its order, each named in both languages', () => {
    expect(SCALE_MODES).toEqual([
      'major',
      'natural_minor',
      'harmonic_minor',
      'melodic_minor',
      'dorian',
      'phrygian',
      'lydian',
      'mixolydian',
      'locrian',
      'major_pentatonic',
      'minor_pentatonic',
      'blues',
    ])
    for (const mode of SCALE_MODES)
      for (const prefix of ['scale.mode.', 'scale.name.', 'scale.short.']) {
        const key = `${prefix}${mode}` as MessageKey
        expect(messages.en[key], key).toBeTruthy()
        expect(messages.fr[key], key).toBeTruthy()
      }
    for (const root of ROOTS) expect(messages.fr[`scale.root.${root}` as MessageKey]).toBeTruthy()
  })

  it('gives the pitch classes of each scale on every root, from the generated degrees', () => {
    expect([...pitchClasses({ root: 5, mode: 'natural_minor' })!].sort((a, b) => a - b)).toEqual([0, 1, 3, 5, 7, 8, 10])
    expect([...pitchClasses({ root: 11, mode: 'major' })!].sort((a, b) => a - b)).toEqual([1, 3, 4, 6, 8, 10, 11])
    expect(pitchClasses(null)).toBeNull()
    for (const { mode, intervals } of SCALE_INTERVALS)
      for (const root of ROOTS) {
        const set = pitchClasses({ root, mode })!
        expect(set.size).toBe(intervals.length)
        expect(set.has(root)).toBe(true)
        for (const pc of set) expect(pc >= 0 && pc < 12).toBe(true)
      }
  })

  it('names a scale in full and in short, in English and French; none is "No scale"', () => {
    const fMinor = { root: 5, mode: 'natural_minor' } as const
    const dDorian = { root: 2, mode: 'dorian' } as const
    expect(scaleName(fMinor, en)).toBe('F natural minor')
    expect(scaleShort(fMinor, en)).toBe('Fm')
    expect(scaleShort(dDorian, en)).toBe('D dor.')
    expect(scaleName(fMinor, fr)).toBe('Fa mineur naturel')
    expect(scaleShort(fMinor, fr)).toBe('Fa m')
    expect(scaleShort({ root: 10, mode: 'major' }, en)).toBe('B♭')
    expect(scaleShort({ root: 6, mode: 'blues' }, fr)).toBe('Fa♯ blues')
    expect(scaleName(null, en)).toBe('No scale')
    expect(scaleName(null, fr)).toBe('Sans gamme')
    expect(scaleShort(undefined, en)).toBe('Scale')
  })

  it('names no scale as chromatic for the command', () => {
    expect(kindOf(null)).toEqual({ root: 0, mode: 'chromatic' })
    expect(kindOf({ root: 7, mode: 'lydian' })).toEqual({ root: 7, mode: 'lydian' })
  })
})

describe('flat or sharp note names by key (#943)', () => {
  const MAJOR = [0, 2, 4, 5, 7, 9, 11]
  const DIATONIC = ['major', 'natural_minor', 'dorian', 'phrygian', 'lydian', 'mixolydian', 'locrian', 'major_pentatonic', 'minor_pentatonic']
  // The picker's spelling of a major key's root decides: a flat in it, or F.
  const flatKey = (tonic: number) => tonic === 5 || en(`scale.root.${tonic}` as MessageKey).includes('♭')

  it('has the picker spell the same roots with a flat in both languages', () => {
    for (const tonic of ROOTS)
      expect(fr(`scale.root.${tonic}` as MessageKey).includes('♭'), String(tonic)).toBe(en(`scale.root.${tonic}` as MessageKey).includes('♭'))
  })
  it('follows the picker spelling of the key signature for the twelve tonics of every diatonic and pentatonic scale', () => {
    for (const mode of DIATONIC)
      for (const root of ROOTS) {
        const scale = { root, mode } as Parameters<typeof usesFlats>[0]
        const notes = [...pitchClasses(scale)!]
        // The major keys that hold all the notes of the scale (from the protocol degrees); the scale's own is among them.
        const keys = ROOTS.filter((tonic) => notes.every((n) => MAJOR.some((d) => (tonic + d) % 12 === n)))
        // A pentatonic scale fits several keys: it reads like the heptatonic scale it comes from.
        const parent = mode === 'major_pentatonic' ? 'major' : mode === 'minor_pentatonic' ? 'natural_minor' : mode
        if (parent === mode) expect(keys.length, `${root} ${mode}`).toBe(1)
        const own = keys.length === 1 ? keys[0] : -1
        const expected = en(`scale.root.${root}` as MessageKey).includes('♭') || (parent === mode && own !== 1 && own !== 6 && flatKey(own))
        if (parent === mode) expect(usesFlats(scale, en), `${root} ${mode}`).toBe(expected)
        else expect(usesFlats(scale, en), `${root} ${mode}`).toBe(usesFlats({ root, mode: parent } as typeof scale, en))
        expect(usesFlats(scale, fr), `${root} ${mode} fr`).toBe(usesFlats(scale, en))
      }
  })
  it('answers for every offered scale on every root, and names the keys of the issue', () => {
    for (const mode of SCALE_MODES) for (const root of ROOTS) expect(typeof usesFlats({ root, mode }, en)).toBe('boolean')
    expect(usesFlats({ root: 5, mode: 'natural_minor' }, en)).toBe(true)
    expect(usesFlats({ root: 5, mode: 'major' }, en)).toBe(true)
    expect(usesFlats({ root: 2, mode: 'natural_minor' }, en)).toBe(true)
    expect(usesFlats({ root: 1, mode: 'major' }, en)).toBe(false)
    expect(usesFlats({ root: 10, mode: 'natural_minor' }, en)).toBe(true)
    expect(usesFlats({ root: 3, mode: 'natural_minor' }, en)).toBe(true)
    expect(usesFlats({ root: 8, mode: 'mixolydian' }, en)).toBe(true)
    expect(usesFlats({ root: 1, mode: 'mixolydian' }, en)).toBe(false)
    expect(usesFlats({ root: 8, mode: 'natural_minor' }, en)).toBe(true)
    expect(usesFlats({ root: 10, mode: 'locrian' }, en)).toBe(true)
    expect(usesFlats({ root: 3, mode: 'phrygian' }, en)).toBe(true)
    expect(usesFlats({ root: 6, mode: 'natural_minor' }, en)).toBe(false)
    expect(usesFlats({ root: 6, mode: 'major' }, en)).toBe(false)
    expect(usesFlats({ root: 9, mode: 'natural_minor' }, en)).toBe(false)
    expect(usesFlats({ root: 7, mode: 'major' }, en)).toBe(false)
    expect(usesFlats({ root: 0, mode: 'major' }, en)).toBe(false)
    expect(usesFlats(null, en)).toBe(false)
  })
})
