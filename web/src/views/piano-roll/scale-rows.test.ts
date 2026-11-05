import { describe, expect, it } from 'vitest'
import { pitchClasses } from '../../lib/scale'
import { rowShaded } from './draw'

describe('the rows of the piano roll and the project scale (#695, #511)', () => {
  it('shades the black keys without a scale, the notes out of the scale with one', () => {
    const shaded = (scale: Parameters<typeof rowShaded>[1]) => Array.from({ length: 12 }, (_, pc) => rowShaded(60 + pc, scale))
    // C4 to B4 without a scale: the black keys.
    expect(shaded(null)).toEqual([false, true, false, true, false, false, true, false, true, false, true, false])
    // F minor: F G A♭ B♭ C D♭ E♭ are open, D, E, G♭, A and B shaded.
    const fMinor = { classes: pitchClasses({ root: 5, mode: 'natural_minor' })! }
    expect(shaded(fMinor)).toEqual([false, false, true, false, true, false, true, false, false, true, false, true])
    // C major is the white keys, in every octave.
    const cMajor = { classes: pitchClasses({ root: 0, mode: 'major' })! }
    for (const pitch of [0, 25, 61, 127]) expect(rowShaded(pitch, cMajor)).toBe(rowShaded(pitch, null))
  })
})
