import { describe, expect, it } from 'vitest'
import { lengthText, positionText, snapText } from './gesture-label'
import { PPQ } from '../views/transport/time'
import { translate } from './i18n/app'
import type { Lang } from './i18n/lang'

const sig = { numerator: 4, denominator: 4 }
const bar = PPQ * 4
const tIn = (lang: Lang) => (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate(lang, key, vars)
const en = tIn('en')
const names = { bar: 'Bar', beat: 'Beat' }

describe('gesture label text (#612)', () => {
  it('writes a position as bar and beat from 1', () => {
    expect(positionText(en, sig, 0)).toBe('Bar 1, beat 1')
    expect(positionText(en, sig, 2 * bar + PPQ * 1.5)).toBe('Bar 3, beat 2.5')
    expect(positionText(en, sig, -5)).toBe('Bar 1, beat 1')
  })

  it('writes a length in bars and beats, with the singular and without a zero part', () => {
    expect(lengthText(en, sig, bar)).toBe('Length 1 bar')
    expect(lengthText(en, sig, 2 * bar + PPQ)).toBe('Length 2 bars, 1 beat')
    expect(lengthText(en, sig, PPQ * 2.5)).toBe('Length 2.5 beats')
    expect(lengthText(en, sig, 0)).toBe('Length 0 beats')
    expect(lengthText(en, sig, 1)).toBe('Length 0.001 beats')
    expect(lengthText(tIn('fr'), sig, 3 * bar)).toBe('Longueur 3 mesures')
  })

  it('names the snap in force, or Free with no step', () => {
    expect(snapText(en, sig, bar, names)).toBe('Snap: Bar')
    expect(snapText(en, sig, PPQ, names)).toBe('Snap: Beat')
    expect(snapText(en, sig, PPQ / 2, names)).toBe('Snap: 1/8')
    expect(snapText(en, sig, PPQ / 4, names)).toBe('Snap: 1/16')
    expect(snapText(en, sig, PPQ / 8, names)).toBe('Snap: 1/32')
    // A 6/8 bar counts its beats in eighths: a quarter of a beat is a thirty-second.
    expect(snapText(en, { numerator: 6, denominator: 8 }, PPQ / 8, names)).toBe('Snap: 1/32')
    expect(snapText(en, sig, 2 * bar, names)).toBe('Snap: 2 bars')
    expect(snapText(en, sig, null, names)).toBe('Snap: Free')
    expect(snapText(tIn('fr'), sig, null, names)).toBe('Magnétisme : Libre')
  })

  it('names a triplet step as the piano roll names it (#711), and as a fraction without those names', () => {
    const roll = { ...names, eighthTriplet: '1/8 triplet', sixteenthTriplet: '1/16 triplet' }
    expect(snapText(en, sig, PPQ / 3, roll)).toBe('Snap: 1/8 triplet')
    expect(snapText(en, sig, PPQ / 6, roll)).toBe('Snap: 1/16 triplet')
    expect(snapText(en, sig, PPQ / 4, roll)).toBe('Snap: 1/16')
    expect(snapText(en, sig, PPQ / 3, names)).toBe('Snap: 1/12')
  })
})
