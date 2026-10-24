import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { CommandError } from '../generated/protocol'
import { messages } from '../i18n'
import { refusalOf } from './refusal'

const ERRORS: CommandError[] = [
  { code: 'not_found', entity: 'track', id: '1' },
  { code: 'invalid', field: 'name', message: 'too long' },
  { code: 'limit', entity: 'clips per track', max: 100000 },
  { code: 'unsupported', command: 'x' },
  { code: 'folder', error: { reason: 'not_a_project' } },
]

describe('refusalOf', () => {
  it('has a text in both languages for every refusal code', () => {
    for (const error of ERRORS) {
      const { key, whatKey } = refusalOf(error)
      for (const lang of ['en', 'fr'] as const) {
        expect(messages[lang][key], `${lang} ${error.code}`).toBeTruthy()
        if (whatKey) expect(messages[lang][whatKey], `${lang} ${whatKey}`).toBeTruthy()
      }
    }
    expect(new Set(ERRORS.map((e) => e.code)).size).toBe(5)
  })

  it('names what a limit counts, and falls back for an entity it does not know', () => {
    expect(refusalOf({ code: 'limit', entity: 'clips per track', max: 7 })).toEqual({ key: 'refusal.limit', vars: { max: 7 }, whatKey: 'refusal.entity.clipsPerTrack' })
    expect(refusalOf({ code: 'limit', entity: 'something new', max: 3 }).whatKey).toBe('refusal.entity.other')
  })

  it('knows every entity the commands crate can name in a limit', () => {
    const dir = `${process.cwd()}/../crates/commands/src`
    const sources = readdirSync(dir).filter((f) => f.endsWith('.rs')).map((f) => readFileSync(`${dir}/${f}`, 'utf8')).join('\n')
    // `entity: "x".into()` and the `("x", count, ...)` / `check_total("x", ...)` forms.
    const named = new Set<string>()
    for (const m of sources.matchAll(/entity: "([^"]+)"\.into\(\)/g)) named.add(m[1])
    for (const m of sources.matchAll(/check_total\(\s*"([^"]+)"/g)) named.add(m[1])
    for (const m of sources.matchAll(/\(\s*"(clips|placements|lanes)",\s*counts\./g)) named.add(m[1])
    for (const entity of ['tracks', 'scenes', 'clips', 'notes', 'notes per clip', 'devices', 'automation points', 'placements', 'placements per track', 'clips per track', 'lanes', 'export seconds']) {
      expect(named.has(entity), `the crate still names ${entity}`).toBe(true)
    }
    // Entities that are not limits (not_found: track, clip, scene...) are not in the table.
    const limits = ['tracks', 'scenes', 'clips', 'notes', 'notes per clip', 'devices', 'automation points', 'placements', 'placements per track', 'clips per track', 'lanes', 'export seconds']
    for (const entity of limits) expect(refusalOf({ code: 'limit', entity, max: 1 }).whatKey, entity).not.toBe('refusal.entity.other')
  })

  it('never leaves a code without a text', () => {
    expect(refusalOf({ code: 'from_the_future' } as unknown as CommandError).key).toBe('refusal.invalid')
  })
})
