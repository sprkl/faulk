import { describe, expect, it } from 'vitest'
import { DEVICE_NAME_KEYS } from './app'
import { defaultLang, landingMessages, messages, translate } from './index'
import { LANDING_DEVICE_NAME_KEYS } from './landing'

/** Every catalogue of the app: the DAW and the landing page. */
const catalogues = { app: messages, landing: landingMessages } as const

describe('i18n', () => {
  for (const [name, catalogue] of Object.entries(catalogues)) {
    it(`${name}: has the same keys in every language, none empty`, () => {
      const en = Object.keys(catalogue.en).sort()
      expect(Object.keys(catalogue.fr).sort()).toEqual(en)
      for (const lang of ['en', 'fr'] as const) {
        for (const [key, text] of Object.entries(catalogue[lang])) expect(text.trim(), `${lang} ${key}`).not.toBe('')
      }
    })

    it(`${name}: uses the same placeholders in every language`, () => {
      const vars = (s: string) => (s.match(/\{\w+\}/g) ?? []).sort()
      const en = catalogue.en as Record<string, string>
      const fr = catalogue.fr as Record<string, string>
      for (const key of Object.keys(en)) expect(vars(fr[key]), key).toEqual(vars(en[key]))
    })
  }

  it('shows each device name as one name, the same in every language', () => {
    const lists = [
      [messages, DEVICE_NAME_KEYS],
      [landingMessages, LANDING_DEVICE_NAME_KEYS],
    ] as const
    for (const [catalogue, keys] of lists) {
      expect(keys.length).toBeGreaterThan(0)
      const en = catalogue.en as Record<string, string>
      const fr = catalogue.fr as Record<string, string>
      for (const key of keys) {
        expect(en[key]?.trim(), key).toBeTruthy()
        expect(fr[key], key).toBe(en[key])
      }
    }
  })

  it('fills placeholders', () => {
    expect(translate('en', 'engine.latency', { ms: 12 })).toBe('~12 ms (estimated)')
    expect(translate('fr', 'helper.refused', { n: 3 })).toBe('Code incorrect. Encore 3 essais.')
  })

  it('picks the stored language, else the browser one', () => {
    expect(defaultLang('fr', 'en-US')).toBe('fr')
    expect(defaultLang('en', 'fr-FR')).toBe('en')
    expect(defaultLang(null, 'fr-BE')).toBe('fr')
    expect(defaultLang(null, 'FR')).toBe('fr')
    expect(defaultLang(null, 'de-DE')).toBe('en')
    expect(defaultLang('xx', undefined)).toBe('en')
    expect(defaultLang(null, '')).toBe('en')
  })
})
