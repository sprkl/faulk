import { describe, expect, it } from 'vitest'
import { mergeRoots, neverIfEmpty, sameDefinition } from './schema-merge.mjs'

const id = { type: 'string', description: 'Identifier of a track.' }

describe('mergeRoots', () => {
  it('declares a definition shared by two roots once', () => {
    const defs = mergeRoots({
      A: { $schema: 'x', title: 'A', type: 'object', $defs: { TrackId: id } },
      B: { title: 'B', type: 'object', $defs: { TrackId: { description: id.description, type: 'string' } } },
    })
    expect(Object.keys(defs).sort()).toEqual(['A', 'B', 'TrackId'])
    expect(defs.TrackId).toEqual(id)
  })

  it('titles each root by its name and drops its $schema and $defs', () => {
    const defs = mergeRoots({ CommandOutput: { $schema: 'x', title: 'Output', type: 'object', $defs: {} } })
    expect(defs.CommandOutput).toEqual({ title: 'CommandOutput', type: 'object' })
  })

  it('throws when two roots define a name differently', () => {
    expect(() =>
      mergeRoots({
        A: { type: 'object', $defs: { Author: { type: 'string' } } },
        B: { type: 'object', $defs: { Author: { type: 'string', description: 'other' } } },
      }),
    ).toThrow('schema definition Author differs between roots')
  })

  it("keeps a root's own titled copy over the copy embedded in another root", () => {
    const command = { type: 'object', description: 'A command.' }
    for (const order of [
      ['Command', 'ClientMessage'],
      ['ClientMessage', 'Command'],
    ]) {
      const roots: Record<string, Record<string, unknown>> = {
        Command: { $schema: 'x', title: 'Command', ...command },
        ClientMessage: { type: 'object', $defs: { Command: command } },
      }
      const defs = mergeRoots(Object.fromEntries(order.map((k) => [k, roots[k]])))
      expect(defs.Command).toEqual({ ...command, title: 'Command' })
    }
  })

  it('turns an empty domain into never, in roots and definitions', () => {
    const empty = { description: 'No commands yet.', not: {} }
    const defs = mergeRoots({ A: { type: 'object', $defs: { TrackCommand: empty } } })
    expect(defs.TrackCommand).toEqual({ description: 'No commands yet.', tsType: 'never' })
  })
})

describe('neverIfEmpty', () => {
  it('leaves a schema with more than an empty not alone', () => {
    for (const def of [{ not: {}, type: 'string' }, { not: { type: 'string' } }, { type: 'object' }]) {
      expect(neverIfEmpty(def)).toBe(def)
    }
  })
})

describe('sameDefinition', () => {
  it('ignores key order and title, nothing else', () => {
    expect(sameDefinition({ a: 1, b: { c: [1, { d: 2, e: 3 }] } }, { b: { c: [1, { e: 3, d: 2 }] }, a: 1, title: 'T' })).toBe(true)
    expect(sameDefinition({ a: 1 }, { a: 1, description: 'x' })).toBe(false)
    expect(sameDefinition({ c: [1, 2] }, { c: [2, 1] })).toBe(false)
  })
})
