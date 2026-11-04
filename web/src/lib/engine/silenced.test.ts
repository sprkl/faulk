import { describe, expect, it } from 'vitest'
import { translate } from '../i18n/index'
import { silencedDuration } from './silenced'

const say = (frames: number, rate: number, lang: 'en' | 'fr') => {
  const d = silencedDuration(frames, rate, lang)
  return translate(lang, d.key, { n: d.n })
}

describe('silencedDuration (#667)', () => {
  it('says under 1 ms for a few frames, never 0 ms', () => {
    expect(say(3, 48_000, 'en')).toBe('under 1 ms')
    expect(say(3, 48_000, 'fr')).toBe('moins d’1 ms')
  })

  it('gives whole milliseconds below one second, then seconds with one decimal in the language', () => {
    expect(say(48, 48_000, 'en')).toBe('1 ms')
    expect(say(4_800, 48_000, 'en')).toBe('100 ms')
    expect(say(47_975, 48_000, 'en')).toBe('999 ms')
    expect(say(47_999, 48_000, 'en')).toBe('1 s')
    expect(say(48_000, 48_000, 'en')).toBe('1 s')
    expect(say(62_400, 48_000, 'en')).toBe('1.3 s')
    expect(say(62_400, 48_000, 'fr')).toBe('1,3 s')
    expect(say(44_100, 44_100, 'en')).toBe('1 s')
  })

  it('counts an unusable rate as 48 kHz', () => {
    expect(say(4_800, 0, 'en')).toBe('100 ms')
    expect(say(4_800, NaN, 'en')).toBe('100 ms')
  })
})
