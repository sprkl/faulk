import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { LANDING_DEVICE_NAME_KEYS, landingMessages, NON_LATENCY_MS_KEYS, type LandingKey } from '../i18n/landing'

const dir = new URL('./', import.meta.url)
const components = readdirSync(dir).filter((f) => f.endsWith('.svelte'))

/** Text a component shows without going through the catalogue. */
function literalText(source: string): string[] {
  let s = source
    .replace(/<script[\s\S]*?<\/script>/g, '')
    .replace(/<style[\s\S]*?<\/style>/g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
  // Drop `{...}` expressions, innermost first (template literals nest braces).
  for (let prev = ''; prev !== s;) [prev, s] = [s, s.replace(/\{[^{}]*\}/g, '')]
  return s
    .replace(/<[^>]*>/g, '\n')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

describe('landing components', () => {
  it('exist', () => expect(components.length).toBeGreaterThan(5))

  for (const file of components) {
    const source = readFileSync(new URL(file, dir), 'utf8')

    it(`${file}: shows no text outside the catalogue`, () => {
      // The decorative wordmark is the brand name, the same in every language, and aria-hidden.
      expect(literalText(source).filter((text) => text !== 'FAULK')).toEqual([])
    })

    it(`${file}: has no literal accessible names`, () => {
      expect(source.match(/\b(aria-label|alt|title|placeholder)="[^"{][^"]*"/g) ?? []).toEqual([])
    })
  }
})

describe('landing device names', () => {
  it('lists every library card name as a device name (one name in every language)', () => {
    const source = readFileSync(new URL('Library.svelte', dir), 'utf8')
    const names = [...source.matchAll(/\bname:\s*['"`]([\w.]+)['"`]/g)].map((m) => m[1])
    expect(names.length).toBeGreaterThan(0)
    for (const name of names) expect(LANDING_DEVICE_NAME_KEYS, name).toContain(name)
  })
})

describe('landing honesty', () => {
  const entries = (['en', 'fr'] as const).flatMap((lang) =>
    Object.entries(landingMessages[lang]).map(([key, text]) => ({ lang, key: key as LandingKey, text })),
  )

  it('states no latency figure (only module parameter values may contain milliseconds)', () => {
    const withMs = entries.filter((e) => /\d\s*ms\b/.test(e.text) && !NON_LATENCY_MS_KEYS.includes(e.key))
    expect(withMs).toEqual([])
  })

  it('offers the Windows download and labels the macOS one as coming soon', () => {
    expect(landingMessages.en['boost.windows']).toContain('Download')
    expect(landingMessages.fr['boost.windows']).toContain('Télécharger')
    expect(landingMessages.en['boost.macos']).toContain('coming soon')
    expect(landingMessages.fr['boost.macos']).toContain('bientôt')
  })

  it('marks the Atelier and the library as not available yet', () => {
    expect(landingMessages.en['atelier.eyebrow']).toContain('COMING')
    expect(landingMessages.fr['atelier.eyebrow']).toContain('À VENIR')
    expect(landingMessages.en['library.browse']).toContain('coming soon')
    expect(landingMessages.fr['library.browse']).toContain('bientôt')
  })
})
