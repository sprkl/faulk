// The audio import pipeline's WebAssembly module (crates/audio-import-wasm, AU-1): its C ABI
// (`ai_*`) behind a small class. One job at a time per instance; a panic of the module traps
// it, after which the instance is dropped (`broken`) and a new one is made.

/** Why an import failed (`ImportError::code` in crates/audio-import). */
export type ImportErrorCode =
  | 'unsupported_format'
  | 'unsupported_codec'
  | 'corrupt'
  | 'empty'
  | 'bad_sample_rate'
  | 'bad_channels'
  | 'too_long'
  /** Decodes to more than a project keeps of one file (`MAX_DECODED_BYTES`, #780). */
  | 'too_large'
  | 'io'
  /** Not the module's: the user cancelled, or the store could not write. */
  | 'cancelled'
  | 'store'
  /** Not the module's: the page has no Worker to run the import in. */
  | 'unavailable'

const CODES: Record<number, ImportErrorCode> = {
  [-1]: 'unsupported_format',
  [-2]: 'unsupported_codec',
  [-3]: 'corrupt',
  [-4]: 'empty',
  [-5]: 'bad_sample_rate',
  [-6]: 'bad_channels',
  [-7]: 'too_long',
  [-8]: 'io',
  [-9]: 'too_large',
}

export class ImportError extends Error {
  constructor(
    readonly code: ImportErrorCode,
    message: string,
  ) {
    super(message)
    this.name = 'ImportError'
  }
}

/** Reads bytes `[offset, offset + length)` of the file now (synchronously), fewer at its end. */
export type ReadSync = (offset: number, length: number) => Uint8Array

interface Exports {
  memory: WebAssembly.Memory
  ai_inbox(len: number): number
  ai_hash_begin(): void
  ai_hash_push(len: number): number
  ai_hash_end(): number
  ai_open(lenLo: number, lenHi: number, hintLen: number): number
  ai_next(): number
  ai_block(): number
  ai_block_len(): number
  ai_position(): number
  ai_finish(): number
  ai_rate(): number
  ai_channels(): number
  ai_frames(): number
  ai_layout(headLen: number, lenLo: number, lenHi: number, rate: number, channels: number, frames: number): number
  ai_layout_candidate(headLen: number, lenLo: number, lenHi: number): number
  ai_cache_header(rate: number, channels: number, frames: number): number
  ai_cache_valid(headLen: number, fileLen: number, rate: number, channels: number, frames: number): number
  ai_peaks(): number
  ai_peaks_len(): number
  ai_peaks_valid(headLen: number, fileLen: number, rate: number, channels: number, frames: number): number
  ai_error(): number
  ai_error_len(): number
  ai_limit(maxBytes: number): void
  ai_max_decoded_bytes(): number
}

/** What decoding found (the clip's metadata but the name and the hash). */
export interface MediaInfo {
  sample_rate: number
  channels: number
  frames: number
}

/** `ai_next`: a block is ready. */
const BLOCK = 1
/** `ai_next`: the end of the stream. */
const END = 0

/** The bytes of the PCM cache's header. */
export const CACHE_HEADER_BYTES = 32

/** The bytes of a peaks file's header (`peaks::HEADER_BYTES` of crates/audio-import). */
export const PEAKS_HEADER_BYTES = 32

/** How much of the start of a file `layout` reads (`HEAD_BYTES` of crates/audio-import). */
export const HEAD_BYTES = 64 * 1024

/**
 * The most bytes of `f32` samples one file may decode to (`project::limits::MAX_DECODED_BYTES`, #780: 16 GiB),
 * the most its PCM cache holds; a file past it is refused as `too_large`. The module's own value is checked equal.
 */
export const MAX_DECODED_BYTES = 16 * 2 ** 30

const lo = (n: number) => n >>> 0
const hi = (n: number) => Math.floor(n / 2 ** 32)

export class ImportModule {
  private readonly exports: Exports
  private reader: ReadSync | null = null
  /** Set once the module trapped: its state is lost, use a new instance. */
  broken = false

  /**
   * `maxDecodedBytes` lowers the cap on the decoded size of every job of this instance (tests reach the refusal
   * without decoding 16 GiB); the module never takes one above `MAX_DECODED_BYTES`.
   */
  constructor(
    module: WebAssembly.Module,
    private readonly maxDecodedBytes?: number,
  ) {
    const env = {
      ai_read: (offset: number, length: number, dst: number): number => {
        try {
          const bytes = this.reader?.(offset, length)
          if (!bytes) return -1
          new Uint8Array(this.exports.memory.buffer, dst, bytes.length).set(bytes)
          return bytes.length
        } catch {
          return -1
        }
      },
    }
    this.exports = new WebAssembly.Instance(module, { env }).exports as unknown as Exports
  }

  /** Runs a call of the module; a trap breaks the instance and is reported as a damaged file. */
  private guard<T>(call: () => T): T {
    if (this.broken) throw new ImportError('corrupt', 'the import module stopped')
    try {
      return call()
    } catch (error) {
      if (error instanceof ImportError) throw error
      this.broken = true
      throw new ImportError('corrupt', `the decoder stopped on this file (${error instanceof Error ? error.message : String(error)})`)
    }
  }

  private inbox(bytes: Uint8Array): number {
    const ptr = this.exports.ai_inbox(bytes.length)
    if (ptr === 0 && bytes.length > 0) throw new ImportError('io', 'out of memory')
    new Uint8Array(this.exports.memory.buffer, ptr, bytes.length).set(bytes)
    return bytes.length
  }

  private failure(code: number): ImportError {
    const text = new TextDecoder().decode(new Uint8Array(this.exports.memory.buffer, this.exports.ai_error(), this.exports.ai_error_len()))
    return new ImportError(CODES[code] ?? 'corrupt', text || `import failed with code ${code}`)
  }

  hashBegin() {
    this.guard(() => this.exports.ai_hash_begin())
  }

  hashPush(bytes: Uint8Array) {
    this.guard(() => {
      if (this.exports.ai_hash_push(this.inbox(bytes)) !== 0) throw new ImportError('io', 'no hash started')
    })
  }

  /** The SHA-256, in hex. */
  hashEnd(): string {
    return this.guard(() => {
      const ptr = this.exports.ai_hash_end()
      if (ptr === 0) throw new ImportError('io', 'no hash started')
      return Array.from(new Uint8Array(this.exports.memory.buffer, ptr, 32), (b) => b.toString(16).padStart(2, '0')).join('')
    })
  }

  /** Opens a file of `size` bytes, read through `reader`, for decoding. */
  open(reader: ReadSync, size: number, extension: string) {
    this.reader = reader
    this.guard(() => {
      const hint = this.inbox(new TextEncoder().encode(extension))
      // The cap holds for the next opening only: set again for each job.
      if (this.maxDecodedBytes !== undefined) this.exports.ai_limit(this.maxDecodedBytes)
      const code = this.exports.ai_open(lo(size), hi(size), hint)
      if (code !== 0) throw this.failure(code)
    })
  }

  /** Decodes the next packet: its interleaved samples (a view of the module's memory, valid until the next call), or `null` at the end. */
  next(): Float32Array | null {
    return this.guard(() => {
      const code = this.exports.ai_next()
      if (code === END) return null
      if (code !== BLOCK) throw this.failure(code)
      return new Float32Array(this.exports.memory.buffer, this.exports.ai_block(), this.exports.ai_block_len())
    })
  }

  /** The furthest byte decoding has read. */
  position(): number {
    return this.exports.ai_position()
  }

  finish(): MediaInfo {
    return this.guard(() => {
      const code = this.exports.ai_finish()
      this.reader = null
      if (code !== 0) throw this.failure(code)
      return { sample_rate: this.exports.ai_rate(), channels: this.exports.ai_channels(), frames: this.exports.ai_frames() }
    })
  }

  /** Whether a file whose first bytes are `head`, of `size` bytes and this metadata, is read directly (no PCM cache). */
  direct(head: Uint8Array, size: number, info: MediaInfo): boolean {
    return this.guard(() => this.exports.ai_layout(this.inbox(head), lo(size), hi(size), info.sample_rate, info.channels, info.frames) === 1)
  }

  /** Whether a file whose first bytes are `head` looks read directly, before decoding (a WAV or AIFF of plain samples). */
  maybeDirect(head: Uint8Array, size: number): boolean {
    return this.guard(() => this.exports.ai_layout_candidate(this.inbox(head), lo(size), hi(size)) === 1)
  }

  /** The header of the PCM cache of such a file. */
  cacheHeader(info: MediaInfo): Uint8Array {
    return this.guard(() => {
      const ptr = this.exports.ai_cache_header(info.sample_rate, info.channels, info.frames)
      if (ptr === 0) throw new ImportError('io', 'not the metadata of a file')
      return new Uint8Array(this.exports.memory.buffer, ptr, CACHE_HEADER_BYTES).slice()
    })
  }

  /** The waveform peaks file of the job just finished (AU-2, a copy), or `null` when it gave none. */
  peaks(): Uint8Array | null {
    return this.guard(() => {
      const len = this.exports.ai_peaks_len()
      return len > 0 ? new Uint8Array(this.exports.memory.buffer, this.exports.ai_peaks(), len).slice() : null
    })
  }

  /** Whether a peaks file starting with `head`, of `size` bytes, is the whole peaks file of such a media for this decoder. */
  peaksValid(head: Uint8Array, size: number, info: MediaInfo): boolean {
    return this.guard(() => this.exports.ai_peaks_valid(this.inbox(head), size, info.sample_rate, info.channels, info.frames) === 1)
  }

  /** The most bytes of `f32` samples a file may decode to, as the module holds it. */
  maxDecoded(): number {
    return this.exports.ai_max_decoded_bytes()
  }

  /** Whether a cache file starting with `head`, of `size` bytes, is the whole cache of such a file. */
  cacheValid(head: Uint8Array, size: number, info: MediaInfo): boolean {
    return this.guard(() => this.exports.ai_cache_valid(this.inbox(head), size, info.sample_rate, info.channels, info.frames) === 1)
  }
}
