import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { CommandError } from '../generated/protocol'
import { messages } from '../i18n'
import { ENTITY_KEYS, refusalOf } from './refusal'

const ERRORS: CommandError[] = [
  { code: 'not_found', entity: 'track', id: '1' },
  { code: 'invalid', field: 'name', message: 'too long' },
  { code: 'invalid', field: 'if_occupied', message: '1 placement(s) in the way' },
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

  it('says a place was taken, not that a value was refused, for a command that must carve nothing (#805)', () => {
    expect(refusalOf({ code: 'invalid', field: 'if_occupied', message: 'x' }).key).toBe('refusal.occupied')
    expect(refusalOf({ code: 'invalid', field: 'by', message: 'x' }).key).toBe('refusal.invalid')
  })

  it('names what a limit counts, and falls back for an entity it does not know', () => {
    expect(refusalOf({ code: 'limit', entity: 'clips per track', max: 7 })).toEqual({
      key: 'refusal.limit',
      vars: { max: 7 },
      whatKey: 'refusal.entity.clipsPerTrack',
    })
    expect(refusalOf({ code: 'limit', entity: 'something new', max: 3 }).whatKey).toBe('refusal.entity.other')
  })

  it('knows every entity the commands crate can name in a limit, and only those', () => {
    const dir = `${process.cwd()}/../crates/commands/src`
    const sources = readdirSync(dir)
      .filter((f) => f.endsWith('.rs'))
      .map((f) => readFileSync(`${dir}/${f}`, 'utf8'))
      .join('\n')
    const named = new Set<string>()
    // `CommandError::Limit { entity: "x".into(), ..}`, `check_total("x", ..)` and the `("x", counts.x, ..)` rows of the per-track check.
    for (const m of sources.matchAll(/Limit\s*\{\s*entity:\s*"([^"]+)"/g)) named.add(m[1])
    for (const m of sources.matchAll(/check_total\(\s*"([^"]+)"/g)) named.add(m[1])
    for (const m of sources.matchAll(/\(\s*"([a-z ]+)",\s*counts\./g)) named.add(m[1])
    expect(named.size).toBeGreaterThan(8)
    for (const entity of named) expect(ENTITY_KEYS[entity], `the table lacks the limit entity "${entity}"`).toBeDefined()
    for (const entity of Object.keys(ENTITY_KEYS)) expect(named.has(entity), `the crate no longer names "${entity}"`).toBe(true)
  })

  it('never leaves a code without a text', () => {
    expect(refusalOf({ code: 'from_the_future' } as unknown as CommandError).key).toBe('refusal.invalid')
  })
})
