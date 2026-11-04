// The page's reader of the peaks file (AU-2) against files the WebAssembly import module wrote: the
// TypeScript reader and the Rust writer agree on the layout, and a file not of its media is refused.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import { ImportModule, type MediaInfo } from '../import/abi'
import { BLOCK, DECODER, FACTOR, HEADER_BYTES, LEVELS, parsePeaks, peaksPath, peaksSize } from './format'

const FIXTURES = new URL('../../../../../crates/audio-import/tests/fixtures/', import.meta.url)
const fixture = (name: string) => new Uint8Array(readFileSync(new URL(name, FIXTURES)))

let compiled: WebAssembly.Module
beforeAll(async () => {
  compiled = await WebAssembly.compile(readFileSync(new URL('../import/wasm/import.wasm', import.meta.url)))
})

/** Decodes a fixture with the module: its metadata, its peaks file, and its samples. */
function decoded(name: string): { info: MediaInfo; file: Uint8Array; pcm: Float32Array } {
  const bytes = fixture(name)
  const module = new ImportModule(compiled)
  module.open((offset, length) => bytes.subarray(offset, offset + length), bytes.length, name.split('.').pop()!)
  const blocks: Float32Array[] = []
  for (let block = module.next(); block; block = module.next()) blocks.push(block.slice())
  const info = module.finish()
  const pcm = new Float32Array(blocks.reduce((n, b) => n + b.length, 0))
  blocks.reduce((at, b) => (pcm.set(b, at), at + b.length), 0)
  return { info, file: module.peaks()!, pcm }
}

describe('a peaks file as the page reads it', () => {
  it('holds four levels of the media, whose peaks hold every sample of their blocks', () => {
    for (const name of ['sine-s16.wav', 'sine.mp3', 'sine-48k-mono.flac']) {
      const { info, file, pcm } = decoded(name)
      expect(file.length, name).toBe(peaksSize(info))
      const peaks = parsePeaks(file, info)!
      expect(peaks, name).not.toBeNull()
      expect(peaks.levels.map((l) => l.block)).toEqual([256, 1024, 4096, 16_384])
      for (const level of peaks.levels) {
        expect(level.count).toBe(Math.ceil(info.frames / level.block))
        const outside: number[] = []
        for (let frame = 0; frame < info.frames; frame++) {
          for (let c = 0; c < info.channels; c++) {
            const v = Math.max(-127, Math.min(127, pcm[frame * info.channels + c]! * 127))
            const at = (Math.floor(frame / level.block) * info.channels + c) * 2
            if (!(level.data[at]! <= v && v <= level.data[at + 1]!)) outside.push(frame)
          }
        }
        expect(outside, `${name} level ${level.block}`).toEqual([])
      }
    }
  })

  it('names the decoder the module names, and lives under the cache by the blob', () => {
    const module = new ImportModule(compiled)
    const header = module.cacheHeader({ sample_rate: 44_100, channels: 2, frames: 1 })
    expect(new DataView(header.buffer).getUint32(24, true)).toBe(DECODER)
    expect([HEADER_BYTES, BLOCK, FACTOR, LEVELS]).toEqual([32, 256, 4, 4])
    expect(peaksPath('ab')).toBe('.faulk/cache/peaks/ab.peaks')
  })

  it('refuses a file cut short, longer, of another media, decoder or layout', () => {
    const { info, file } = decoded('sine.mp3')
    expect(parsePeaks(file.slice(0, file.length - 1), info)).toBeNull()
    expect(parsePeaks(Uint8Array.from([...file, 0]), info)).toBeNull()
    expect(parsePeaks(file.slice(0, 20), info)).toBeNull()
    for (let byte = 0; byte < HEADER_BYTES; byte++) {
      expect(
        parsePeaks(
          Uint8Array.from(file, (b, i) => (i === byte ? b ^ 1 : b)),
          info,
        ),
        `byte ${byte}`,
      ).toBeNull()
    }
    expect(parsePeaks(file, { ...info, frames: info.frames + 1 })).toBeNull()
    expect(parsePeaks(file, { ...info, channels: 1 })).toBeNull()
    expect(parsePeaks(file, { ...info, sample_rate: 48_000 })).toBeNull()
    // A view into a larger buffer reads the same.
    const larger = new Uint8Array(file.length + 10)
    larger.set(file, 5)
    expect(parsePeaks(larger.subarray(5, 5 + file.length), info)?.levels[0]!.data).toEqual(parsePeaks(file, info)!.levels[0]!.data)
  })
})
