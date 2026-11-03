import { describe, expect, it } from 'vitest'
import { hasNames, stripNames } from './wasm-names.mjs'

const HEADER = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
const custom = (name: string, payload: number[]) => [0, name.length + 1 + payload.length, name.length, ...[...name].map((c) => c.charCodeAt(0)), ...payload]
// An empty type section, then a function-less module's other sections are not needed: one type section is enough.
const TYPES = [1, 1, 0]

const module = (...sections: number[][]) => new Uint8Array([...HEADER, ...sections.flat()])

describe('stripNames', () => {
  it('removes the name section and keeps every other byte', () => {
    const input = module(TYPES, custom('name', [1, 2, 3]), custom('producers', [9]))
    const output = stripNames(input)
    expect([...output]).toEqual([...module(TYPES, custom('producers', [9]))])
    expect(hasNames(input)).toBe(true)
    expect(hasNames(output)).toBe(false)
  })

  it('removes a name section at the end, and one that is the only section', () => {
    expect([...stripNames(module(TYPES, custom('name', [7, 7])))]).toEqual([...module(TYPES)])
    expect([...stripNames(module(custom('name', [])))]).toEqual([...module()])
  })

  it('leaves a module without names as it is', () => {
    const input = module(TYPES, custom('target_features', [0]))
    expect(stripNames(input)).toBe(input)
  })

  it('does not take another custom section for the names', () => {
    expect(hasNames(module(custom('names', [1]), custom('producers', [])))).toBe(false)
  })

  it('refuses what is not a module or is cut short', () => {
    expect(() => stripNames(new Uint8Array([1, 2, 3]))).toThrow('not a WebAssembly module')
    expect(() => stripNames(new Uint8Array([...HEADER, 1, 5, 0]))).toThrow('truncated')
  })
})
