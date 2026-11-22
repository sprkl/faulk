import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { protoInputs, protoStale, writeStamp } from './proto-stale'

let dir: string
const at = (path: string, seconds: number) => utimesSync(path, seconds, seconds)
const out = () => [join(dir, 'plain.wasm')]
const src = () => [join(dir, 'crate')]
const toolchain = () => join(dir, 'rust-toolchain.toml')
const stamp = () => join(dir, 'toolchain.stamp')
const stale = () => protoStale(out(), src(), toolchain(), stamp())

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'proto-stale-873-'))
  mkdirSync(join(dir, 'crate'))
  writeFileSync(join(dir, 'crate', 'lib.rs'), '')
  writeFileSync(toolchain(), 'channel = "1.100.0"\n')
  writeFileSync(join(dir, 'plain.wasm'), '')
  writeStamp(toolchain(), stamp())
  at(join(dir, 'crate', 'lib.rs'), 100)
  at(join(dir, 'plain.wasm'), 200)
})
afterEach(() => rmSync(dir, { recursive: true, force: true }))

describe('protoStale', () => {
  it('is fresh when built with this toolchain and newer than the sources', () => {
    expect(stale()).toBe(false)
  })
  it('is stale when an output is missing', () => {
    rmSync(join(dir, 'plain.wasm'))
    expect(stale()).toBe(true)
  })
  it('is stale when a crate file is newer', () => {
    at(join(dir, 'crate', 'lib.rs'), 300)
    expect(stale()).toBe(true)
  })
  it('is stale when rust-toolchain.toml changed, whatever the mtimes say', () => {
    writeFileSync(toolchain(), 'channel = "1.101.0"\n')
    at(toolchain(), 50)
    expect(stale()).toBe(true)
  })
  it('is stale when there is no stamp (outputs built before it existed)', () => {
    rmSync(stamp())
    expect(stale()).toBe(true)
  })
  it('is fresh again once the stamp is rewritten', () => {
    writeFileSync(toolchain(), 'channel = "1.101.0"\n')
    writeStamp(toolchain(), stamp())
    expect(stale()).toBe(false)
  })
  it('a touch of the toolchain file without a change does not rebuild', () => {
    at(toolchain(), 900)
    expect(stale()).toBe(false)
  })
})

describe('protoInputs', () => {
  it('names the real toolchain file, the crates, and a stamp next to the outputs', () => {
    const { toolchain: t, sources, outputs, stamp: s } = protoInputs()
    expect(existsSync(t)).toBe(true)
    expect(readFileSync(t, 'utf8')).toContain('channel')
    expect(sources.every((p) => existsSync(p))).toBe(true)
    expect(s.startsWith(outputs[0].slice(0, outputs[0].lastIndexOf('/')))).toBe(true)
  })
})
