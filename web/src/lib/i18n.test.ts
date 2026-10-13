import { describe, expect, it } from 'vitest'
import { defaultLang, messages, translate } from './i18n'

describe('i18n', () => {
  it('has the same keys in every language, none empty', () => {
    const en = Object.keys(messages.en).sort()
    expect(Object.keys(messages.fr).sort()).toEqual(en)
    for (const lang of ['en', 'fr'] as const) {
      for (const [key, text] of Object.entries(messages[lang])) expect(text, `${lang} ${key}`).not.toBe('')
    }
  })

  it('uses the same placeholders in every language', () => {
    const vars = (s: string) => (s.match(/\{\w+\}/g) ?? []).sort()
    for (const key of Object.keys(messages.en) as (keyof typeof messages.en)[]) {
      expect(vars(messages.fr[key]), key).toEqual(vars(messages.en[key]))
    }
  })

  it('fills placeholders', () => {
    expect(translate('en', 'engine.latency', { ms: 12 })).toBe('~12 ms (estimated)')
    expect(translate('fr', 'helper.refused', { n: 3 })).toBe('Code incorrect. Encore 3 essais.')
  })

  it('picks the stored language, else the browser one', () => {
    expect(defaultLang('fr', 'en-US')).toBe('fr')
    expect(defaultLang(null, 'fr-BE')).toBe('fr')
    expect(defaultLang(null, 'de-DE')).toBe('en')
    expect(defaultLang('xx', undefined)).toBe('en')
  })
})
