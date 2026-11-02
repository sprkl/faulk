import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { dayName, dayParts, keyName } from './format'

const dir = new URL('./', import.meta.url)
const components = readdirSync(dir).filter((f) => f.endsWith('.svelte'))

/** Text a component shows without going through the catalogue (same rule as the landing page). */
function literalText(source: string): string[] {
  let s = source
    .replace(/<script[\s\S]*?<\/script>/g, '')
    .replace(/<style[\s\S]*?<\/style>/g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\{#snippet[^}]*\}|\{\/snippet\}|\{@render[^}]*\}/g, '')
  for (let prev = ''; prev !== s;) [prev, s] = [s, s.replace(/\{[^{}]*\}/g, '')]
  return s
    .replace(/<[^>]*>/g, '\n')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

describe('journey components', () => {
  it('exist', () => expect(components.length).toBeGreaterThanOrEqual(4))

  for (const file of components) {
    const source = readFileSync(new URL(file, dir), 'utf8')

    it(`${file}: shows no text outside the catalogue (separators aside)`, () => {
      expect(literalText(source).filter((text) => !/^[·./\s]+$/.test(text))).toEqual([])
    })

    it(`${file}: has no literal accessible names`, () => {
      expect(source.match(/\b(aria-label|alt|title|placeholder)="[^"{][^"]*"/g) ?? []).toEqual([])
    })
  }
})

describe('journey format', () => {
  it('names keys and days in both languages', () => {
    expect(keyName({ tonic: 'A', mode: 'minor' }, 'en')).toBe('A minor')
    expect(keyName({ tonic: 'A', mode: 'minor' }, 'fr')).toBe('La mineur')
    expect(keyName({ tonic: 'F#', mode: 'dorian' }, 'fr')).toBe('Fa# dorien')
    expect(keyName({ tonic: 'D', mode: 'dorian' }, 'en')).toBe('D Dorian')
    expect(dayName('2026-11-02', 'en')).toBe('2 November 2026')
    expect(dayName('2026-11-02', 'fr')).toBe('2 novembre 2026')
    expect(dayParts('2026-10-31', 'fr')).toEqual({ day: '31', month: 'OCT. 2026' })
  })
})
