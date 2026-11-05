// The page's cache of waveform peaks (AU-2): each way a media's peaks become known, or do not, with a
// stand-in for the project's place (the file read, the opening's work, a request to make them).
import { describe, expect, it } from 'vitest'
import type { MediaRef } from '../import/pipeline'
import { HEADER_BYTES, peaksSize } from './format'
import { PeaksCache, type PeaksSource } from './peaks.svelte'

const media: MediaRef = { hash: 'a'.repeat(64), sample_rate: 48_000, channels: 1, frames: 1000 }

/** A whole peaks file of `media` (its header as crates/audio-import writes it; peaks of silence). */
function fileOf(m: MediaRef): Uint8Array {
  const bytes = new Uint8Array(peaksSize(m))
  const view = new DataView(bytes.buffer)
  bytes.set(new TextEncoder().encode('FKPEAK01'))
  view.setUint32(8, m.sample_rate, true)
  view.setUint16(12, m.channels, true)
  view.setUint16(14, 4, true)
  view.setUint32(16, m.frames, true)
  view.setUint32(24, 1, true)
  view.setUint16(28, 256, true)
  view.setUint16(30, 4, true)
  expect(bytes.length).toBeGreaterThan(HEADER_BYTES)
  return bytes
}

/** A place whose files, opening and Worker the test drives. */
function place(files: Map<string, Uint8Array> = new Map()) {
  const log: string[] = []
  let epoch = 0
  let settle = () => {}
  let settled = Promise.resolve()
  let onMake: (m: MediaRef) => void = () => {}
  const source: PeaksSource = {
    read: async (hash) => {
      log.push(`read ${hash.slice(0, 1)}`)
      return files.get(hash) ?? null
    },
    settled: () => {
      log.push('settled')
      return settled
    },
    make: async (m) => {
      log.push(`make ${m.hash.slice(0, 1)}`)
      onMake(m)
    },
    epoch: () => epoch,
  }
  return {
    source,
    files,
    log,
    /** The opening's work is under way until `finish` is called. */
    opening() {
      settled = new Promise<void>((resolve) => (settle = resolve))
      return () => settle()
    },
    makes(fn: (m: MediaRef) => void) {
      onMake = fn
    },
    attach() {
      epoch++
    },
  }
}

describe('the peaks of a media, as views ask', () => {
  it('reads them once from the place, and gives them from memory after', async () => {
    const p = place(new Map([[media.hash, fileOf(media)]]))
    const cache = new PeaksCache(p.source)
    expect(cache.of(media).state).toBe('loading')
    expect(cache.of(media).state).toBe('loading')
    await new Promise((r) => setTimeout(r, 0))
    await cache.idle()
    const entry = cache.of(media)
    expect(entry.state).toBe('ready')
    expect(entry.peaks?.frames).toBe(1000)
    expect(p.log).toEqual(['read a'])
  })

  it('waits for the opening that is making them, then reads them, without asking for them', async () => {
    const p = place()
    const finish = p.opening()
    const cache = new PeaksCache(p.source)
    cache.of(media)
    await new Promise((r) => setTimeout(r, 0))
    expect(cache.of(media).state).toBe('loading')
    p.files.set(media.hash, fileOf(media))
    finish()
    await cache.idle()
    expect(cache.of(media).state).toBe('ready')
    expect(p.log).toEqual(['read a', 'settled', 'read a'])
  })

  it('asks for them when the opening did not make them (a file left doubtful), then reads them', async () => {
    const p = place(new Map([[media.hash, fileOf(media).slice(0, 40)]]))
    p.makes((m) => p.files.set(m.hash, fileOf(m)))
    const cache = new PeaksCache(p.source)
    cache.of(media)
    await new Promise((r) => setTimeout(r, 0))
    await cache.idle()
    expect(cache.of(media).state).toBe('ready')
    expect(p.log).toEqual(['read a', 'settled', 'read a', 'make a', 'read a'])
  })

  it('leaves them absent when none can be made, until the next attach tries again', async () => {
    const p = place()
    const cache = new PeaksCache(p.source)
    cache.of(media)
    await new Promise((r) => setTimeout(r, 0))
    await cache.idle()
    expect(cache.of(media).state).toBe('absent')
    // Asked again in the same attach: nothing more is tried.
    await new Promise((r) => setTimeout(r, 0))
    expect(p.log).toEqual(['read a', 'settled', 'read a', 'make a', 'read a'])
    // A new attach, a permission given back or an import of the file (each counts in the epoch): tried again, and found this time.
    p.attach()
    p.files.set(media.hash, fileOf(media))
    expect(cache.of(media).state).toBe('absent')
    await new Promise((r) => setTimeout(r, 0))
    await cache.idle()
    expect(cache.of(media).state).toBe('ready')
  })

  it('takes a file of another media for none, and a failing read for no file', async () => {
    const other = { ...media, frames: 2000 }
    const p = place(new Map([[media.hash, fileOf(other)]]))
    p.source.read = async () => {
      p.log.push('read')
      throw new Error('gone')
    }
    const cache = new PeaksCache(p.source)
    cache.of(media)
    await new Promise((r) => setTimeout(r, 0))
    await cache.idle()
    expect(cache.of(media).state).toBe('absent')
  })

  it('a request that fails leaves the media absent, never a rejection', async () => {
    const p = place()
    p.source.make = () => Promise.reject(new Error('refused'))
    const cache = new PeaksCache(p.source)
    cache.of(media)
    await new Promise((r) => setTimeout(r, 0))
    await cache.idle()
    expect(cache.peek(media.hash)?.state).toBe('absent')
  })

  it('two media load apart, each once', async () => {
    const other = { ...media, hash: 'b'.repeat(64) }
    const p = place(new Map([[media.hash, fileOf(media)]]))
    p.makes((m) => p.files.set(m.hash, fileOf(m)))
    const cache = new PeaksCache(p.source)
    cache.of(media)
    cache.of(other)
    cache.of(other)
    await new Promise((r) => setTimeout(r, 0))
    await cache.idle()
    expect([cache.of(media).state, cache.of(other).state]).toEqual(['ready', 'ready'])
    expect(p.log.filter((l) => l === 'make b')).toEqual(['make b'])
    expect(p.log.filter((l) => l === 'make a')).toEqual([])
  })
})
