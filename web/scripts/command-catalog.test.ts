import { describe, expect, it } from 'vitest'
import { commandCatalog, entryOf, fieldOf, summaryOf, SUMMARY_MAX } from './command-catalog.mjs'

describe('summaryOf', () => {
  it('keeps the first sentence on one line', () => {
    expect(summaryOf('Rename the project. The name is trimmed.\nUndo: rename it back.')).toBe('Rename the project.')
    expect(summaryOf('Move the\nplayhead to `tick`.')).toBe('Move the playhead to `tick`.')
  })
  it('does not stop at a decimal point or an abbreviation', () => {
    expect(summaryOf('Set the tempo to 120.5 bpm, e.g. 96. Undo: back.')).toBe('Set the tempo to 120.5 bpm, e.g. 96.')
  })
  it('stops at the first blank line when there is no full stop', () => {
    expect(summaryOf('Stop everything\n\nMore text.')).toBe('Stop everything')
  })
  it('cuts a long sentence with an ellipsis', () => {
    const long = summaryOf('a'.repeat(400))
    expect(long.length).toBe(SUMMARY_MAX)
    expect(long.endsWith('…')).toBe(true)
  })
})

describe('fieldOf', () => {
  const defs = {
    TrackId: { type: 'string' },
    Theme: {
      oneOf: [
        { const: 'dark', type: 'string' },
        { const: 'light', type: 'string' },
      ],
    },
    Maybe: { anyOf: [{ type: 'integer' }, { type: 'null' }] },
  }
  it('reduces a schema to a kind a text field takes', () => {
    expect(fieldOf('a', { type: 'string' }, true, defs)).toEqual({ name: 'a', required: true, kind: 'string' })
    expect(fieldOf('a', { type: 'boolean' }, false, defs)).toEqual({ name: 'a', required: false, kind: 'boolean' })
    expect(fieldOf('a', { $ref: '#/$defs/TrackId' }, true, defs)).toEqual({ name: 'a', required: true, ref: 'TrackId', kind: 'string' })
    expect(fieldOf('a', { $ref: '#/$defs/Theme' }, true, defs)).toEqual({ name: 'a', required: true, ref: 'Theme', kind: 'enum', options: ['dark', 'light'] })
  })
  it('marks what takes null', () => {
    expect(fieldOf('a', { type: ['integer', 'null'] }, true, defs)).toEqual({ name: 'a', required: true, kind: 'integer', nullable: true })
    expect(fieldOf('a', { $ref: '#/$defs/Maybe' }, true, defs)).toMatchObject({ kind: 'integer', nullable: true })
  })
  it('falls back to JSON for lists, objects and unions', () => {
    expect(fieldOf('a', { type: 'array', items: { type: 'integer' } }, true, defs).kind).toBe('json')
    expect(fieldOf('a', { type: 'object' }, true, defs).kind).toBe('json')
    expect(fieldOf('a', { anyOf: [{ type: 'string' }, { type: 'integer' }] }, true, defs).kind).toBe('json')
  })
})

describe('commandCatalog', () => {
  const rename = {
    name: 'project.rename',
    kind: 'edit',
    description: 'Rename the project. More.',
    destructive: false,
    input_schema: { type: 'object', properties: { b: { type: 'integer' }, a: { type: 'string' } }, required: ['a'] },
  }
  const query = { name: 'project.get', kind: 'query', description: 'Read.', destructive: false, input_schema: { type: 'object', properties: {} } }
  it('lists the commands but the queries, required fields first', () => {
    const list = commandCatalog([rename, query])
    expect(list.map((c) => c.name)).toEqual(['project.rename'])
    expect(list[0].fields.map((f: { name: string }) => f.name)).toEqual(['a', 'b'])
  })
  it('flags a destructive command only', () => {
    expect(entryOf(rename)).not.toHaveProperty('destructive')
    expect(entryOf({ ...rename, destructive: true }).destructive).toBe(true)
  })
})
