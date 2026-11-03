import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { checkSchemaSize, SCHEMA_MAX_BYTES, sectionSizes } from './schema-size.mjs'

describe('the size cap of schema.json (#559)', () => {
  const bundle = { small: { a: 1 }, large: { text: 'x'.repeat(200) } }

  it('passes a file at or under the cap and returns its size in bytes', () => {
    expect(checkSchemaSize('é'.repeat(10), bundle, 20)).toBe(20)
    expect(checkSchemaSize('', bundle, 0)).toBe(0)
  })

  it('refuses a file one byte over the cap, counting bytes and not characters', () => {
    expect(() => checkSchemaSize('x'.repeat(21), bundle, 20)).toThrow('above its cap')
    expect(() => checkSchemaSize('é'.repeat(11), bundle, 21)).toThrow('above its cap')
  })

  it('names every section, largest first, in the refusal', () => {
    const error = (() => {
      try {
        checkSchemaSize('x'.repeat(100), bundle, 10)
      } catch (e) {
        return String(e)
      }
    })()
    expect(error).toMatch(/large: [\d.]+ MB\n\s+small: [\d.]+ MB/)
    expect(sectionSizes(bundle).map(([key]) => key)).toEqual(['large', 'small'])
  })

  it('holds the committed file under the cap', () => {
    // Read at run time, never imported (the ESLint rule of #436).
    const text = readFileSync(new URL('../src/lib/generated/schema.json', import.meta.url), 'utf8')
    expect(checkSchemaSize(text, JSON.parse(text) as Record<string, unknown>)).toBeLessThanOrEqual(SCHEMA_MAX_BYTES)
  })
})
