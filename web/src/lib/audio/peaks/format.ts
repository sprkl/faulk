// The waveform peaks file of a media (AU-2, `.faulk/cache/peaks/<sha256>.peaks`), as the page reads
// it. Written by crates/audio-import (`peaks.rs`, which documents the layout) in the project store's
// Worker; read here on the main thread, which has no import module: the layout is small and fixed,
// and `format.test.ts` checks this reader against files the WebAssembly module wrote.
import type { MediaInfo } from '../import/abi'

/** The first bytes of a peaks file. */
const MAGIC = 'FKPEAK01'
/** The bytes of the header. */
export const HEADER_BYTES = 32
/** Frames of a level-0 block. */
export const BLOCK = 256
/** How many blocks of a level make one of the next. */
export const FACTOR = 4
/** The levels a file holds: blocks of 256, 1 024, 4 096 and 16 384 frames. */
export const LEVELS = 4
/**
 * The decoder's version (`cache::DECODER` of crates/audio-import), which the header carries: a file
 * of another decoder is not shown, it is made again. `format.test.ts` checks it against the module's.
 */
export const DECODER = 1

export const PEAKS_DIR = ['.faulk', 'cache', 'peaks']
export const peaksPath = (hash: string) => `${PEAKS_DIR.join('/')}/${hash}.peaks`

/** One level: `count` peaks of `block` frames, each `channels × [low, high]` in `data` (in -127..127, 127 for 1.0). */
export interface PeakLevel {
  block: number
  count: number
  data: Int8Array
}

export interface Peaks {
  sampleRate: number
  channels: number
  frames: number
  /** Finest first. */
  levels: PeakLevel[]
}

const countOf = (frames: number, level: number) => Math.ceil(frames / (BLOCK * FACTOR ** level))

/** The size of the peaks file of such a media. */
export function peaksSize(info: MediaInfo): number {
  let total = HEADER_BYTES
  for (let level = 0; level < LEVELS; level++) total += countOf(info.frames, level) * 2 * info.channels
  return total
}

/** Reads a peaks file of this media: `null` when it is not whole, not of this media, or of another decoder. */
export function parsePeaks(bytes: Uint8Array, info: MediaInfo): Peaks | null {
  if (bytes.length < HEADER_BYTES || info.channels < 1 || !Number.isSafeInteger(info.frames)) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  for (let i = 0; i < MAGIC.length; i++) if (bytes[i] !== MAGIC.charCodeAt(i)) return null
  const frames = view.getUint32(16, true) + view.getUint32(20, true) * 2 ** 32
  if (
    view.getUint32(8, true) !== info.sample_rate ||
    view.getUint16(12, true) !== info.channels ||
    view.getUint16(14, true) !== LEVELS ||
    frames !== info.frames ||
    view.getUint32(24, true) !== DECODER ||
    view.getUint16(28, true) !== BLOCK ||
    view.getUint16(30, true) !== FACTOR ||
    bytes.length !== peaksSize(info)
  ) {
    return null
  }
  const levels: PeakLevel[] = []
  let at = HEADER_BYTES
  for (let level = 0; level < LEVELS; level++) {
    const count = countOf(frames, level)
    const len = count * 2 * info.channels
    levels.push({ block: BLOCK * FACTOR ** level, count, data: new Int8Array(bytes.buffer, bytes.byteOffset + at, len) })
    at += len
  }
  return { sampleRate: info.sample_rate, channels: info.channels, frames, levels }
}
