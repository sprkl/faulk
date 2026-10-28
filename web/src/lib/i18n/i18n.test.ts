import { describe, expect, it } from 'vitest'
import { DEVICE_NAME_KEYS, features } from './app'
import { defaultLang, landingMessages, messages, translate } from './index'
import { DEVICES } from '../generated/protocol'
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

  describe('features of the DAW', () => {
    for (const [name, feature] of Object.entries(features)) {
      it(`${name}: has the same keys in English and French, none empty`, () => {
        expect(Object.keys(feature.en).length).toBeGreaterThan(0)
        expect(Object.keys(feature.fr).sort()).toEqual(Object.keys(feature.en).sort())
        for (const lang of ['en', 'fr'] as const) {
          for (const [key, text] of Object.entries(feature[lang])) expect(text.trim(), `${lang} ${key}`).not.toBe('')
        }
      })
    }

    it('declares each key in one feature only, and the assembly holds them all', () => {
      const all = Object.values(features).flatMap((feature) => Object.keys(feature.en))
      expect(new Set(all).size).toBe(all.length)
      expect(Object.keys(messages.en).sort()).toEqual([...all].sort())
    })
  })

  it('keeps the names of the built-in devices out of the DAW catalogue (they come from the descriptors)', () => {
    for (const { name } of DEVICES) {
      for (const lang of ['en', 'fr'] as const) {
        const keys = Object.entries(messages[lang]).filter(([, text]) => text === name)
        expect(
          keys.map(([key]) => key).filter((key) => !DEVICE_NAME_KEYS.includes(key as never)),
          `${lang} ${name}`,
        ).toEqual([])
      }
    }
  })

  it('shows each device name as one name, the same in every language', () => {
    const lists = [
      [messages, DEVICE_NAME_KEYS],
      [landingMessages, LANDING_DEVICE_NAME_KEYS],
    ] as const
    // The landing page names its sound objects; the DAW names none since DV-1 (the panels take
    // the name from the device's descriptor).
    expect(LANDING_DEVICE_NAME_KEYS.length).toBeGreaterThan(0)
    for (const [catalogue, keys] of lists) {
      const en = catalogue.en as Record<string, string>
      const fr = catalogue.fr as Record<string, string>
      for (const key of keys) {
        expect(en[key]?.trim(), key).toBeTruthy()
        expect(fr[key], key).toBe(en[key])
      }
    }
  })

  it('has, for every parameter of every device, its label and its tooltip in both languages, and for every enumerated value its label', () => {
    // A device added to the registry without its text fails here, not in the panel (which falls back to the key and the description).
    for (const d of DEVICES) {
      const kind = d.kind.replace(/^faulk\./, '')
      for (const p of d.params) {
        expect(p.description.trim(), `${d.kind} ${p.key} description`).not.toBe('')
        for (const lang of ['en', 'fr'] as const) {
          const catalogue = messages[lang] as Record<string, string>
          expect(catalogue[`${kind}.${p.key}`], `${lang} ${kind}.${p.key}`).toBeTruthy()
          expect(catalogue[`${kind}.${p.key}.hint`], `${lang} ${kind}.${p.key}.hint`).toBeTruthy()
          for (const value of p.values) expect(catalogue[`${kind}.${value}`], `${lang} ${kind}.${value}`).toBeTruthy()
        }
        // One English text for agents and for the tooltip: they cannot drift apart.
        expect((messages.en as Record<string, string>)[`${kind}.${p.key}.hint`], `${kind}.${p.key}`).toBe(p.description)
      }
    }
  })

  it('fills placeholders', () => {
    expect(translate('en', 'engine.latency', { ms: 12 })).toBe('~12 ms (estimated)')
    expect(translate('fr', 'helper.refused', { n: 3 })).toBe('Code incorrect. Encore 3 essais.')
  })

  it('names the real recheck button in the French blocked-access text', () => {
    const recheck = translate('fr', 'helper.recheck')
    expect(translate('fr', 'helper.localAccessBlocked')).toContain(`« ${recheck} »`)
  })

  it('names the real start button in the stream-refused text, in both languages', () => {
    expect(translate('en', 'engine.streamError')).toContain(`"${translate('en', 'start.button')}"`)
    expect(translate('fr', 'engine.streamError')).toContain(`« ${translate('fr', 'start.button')} »`)
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
