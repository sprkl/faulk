// The import pipeline's WebAssembly build (AU-1) under Node, on the reference files of
// crates/audio-import: the same hash and the same PCM (bit for bit, MP3 and AAC within 1e-6) as the native build records in
// `expected.json` (the page and the helper decode a file the same way), then the pipeline over a
// folder in memory: one blob per file, the PCM cache, nothing left behind by a failure or a cancel,
// the caches an opening makes again, and what the store's clean-up leaves to a running import.
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import { FolderBackend } from '../../storage/backend'
import { MemoryDir } from '../../storage/memory-dir'
import { StoreError } from '../../storage/types'
import { CACHE_HEADER_BYTES, HEAD_BYTES, ImportError, ImportModule, MAX_DECODED_BYTES, type ReadSync } from './abi'
import { cachePath, ensureAudioCache, importAudio, importLock, IMPORT_TMP_PREFIX, mediaName, type PipelineDeps } from './pipeline'
import { peaksPath } from '../peaks/format'

const FIXTURES = new URL('../../../../../crates/audio-import/tests/fixtures/', import.meta.url)
const fixture = (name: string) => new Uint8Array(readFileSync(new URL(name, FIXTURES)))
type Record = {
  error?: string
  sha256?: string
  sample_rate?: number
  channels?: number
  frames?: number
  direct?: boolean
  pcm_sha256?: string
  peaks_sha256?: string
}
const expected = JSON.parse(readFileSync(new URL('expected.json', FIXTURES), 'utf8')) as { [name: string]: Record }

let compiled: WebAssembly.Module
beforeAll(async () => {
  compiled = await WebAssembly.compile(readFileSync(new URL('./wasm/import.wasm', import.meta.url)))
})

const readerOf =
  (bytes: Uint8Array): ReadSync =>
  (offset, length) =>
    bytes.subarray(offset, offset + length)

/** The pipeline's dependencies over a fresh instance, reading files whole into memory (Node has no FileReaderSync). */
function deps(extra: Partial<PipelineDeps> = {}): PipelineDeps {
  return {
    // A new instance per job, as the Worker's `importModule` gives.
    module: async () => new ImportModule(compiled),
    readerOf: async (blob) => readerOf(new Uint8Array(await blob.arrayBuffer())),
    ...extra,
  }
}

const sha256 = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex')

/** `sine.webm` whose `OpusHead` says channel mapping family 3 (ambisonics): refused as `unsupported_codec`. */
function ambisonic(): Uint8Array<ArrayBuffer> {
  const bytes = fixture('sine.webm')
  bytes[Buffer.from(bytes).indexOf('OpusHead') + 18] = 3
  return bytes
}

/** MP3 and AAC: compared with the native decode at a tolerance, every other file bit for bit (`REFERENCE_PCM` in tests/it/fixtures.rs). */
const TOLERATED = ['sine.mp3', 'sine.m4a']

describe('the import module, as the page runs it', () => {
  it('gives every reference file the hash and the PCM the native build gives', () => {
    const names = Object.keys(expected)
    expect(names.length).toBe(19)
    for (const name of names) {
      const bytes = fixture(name)
      const record = expected[name]!
      const module = new ImportModule(compiled)
      module.hashBegin()
      for (let at = 0; at < bytes.length; at += 1000) module.hashPush(bytes.subarray(at, at + 1000))
      expect(module.hashEnd(), name).toBe(sha256(bytes))
      const blocks: Float32Array[] = []
      try {
        module.open(readerOf(bytes), bytes.length, name.split('.').pop()!)
        for (let block = module.next(); block; block = module.next()) blocks.push(block.slice())
        const info = module.finish()
        // The waveform peaks (AU-2): the native build's bytes, for every file (MP3 and AAC too).
        expect(sha256(module.peaks()!), `${name} peaks`).toBe(record.peaks_sha256)
        const direct = module.direct(bytes.subarray(0, HEAD_BYTES), bytes.length, info)
        const pcm = new Float32Array(blocks.reduce((n, b) => n + b.length, 0))
        blocks.reduce((at, b) => (pcm.set(b, at), at + b.length), 0)
        const digest = sha256(new Uint8Array(pcm.buffer))
        expect({ ...info, direct, sha256: sha256(bytes) }, name).toEqual({
          sample_rate: record.sample_rate,
          channels: record.channels,
          frames: record.frames,
          direct: record.direct,
          sha256: record.sha256,
        })
        if (TOLERATED.includes(name)) {
          // The native decode, kept whole: equal to a millionth (a last bit of `powf` or `cos`).
          const native = new Float32Array(new Uint8Array(readFileSync(new URL(`${name}.pcm`, FIXTURES))).buffer)
          expect(native.length, name).toBe(pcm.length)
          const worst = pcm.reduce((w, s, i) => Math.max(w, Math.abs(s - native[i]!)), 0)
          console.log(`${name}: page and native decodes differ by ${worst.toExponential(2)} at most`)
          expect(worst, name).toBeLessThanOrEqual(1e-6)
        } else {
          expect(digest, name).toBe(record.pcm_sha256)
        }
      } catch (error) {
        expect(error, name).toBeInstanceOf(ImportError)
        expect((error as ImportError).code, name).toBe(record.error)
      }
    }
  })

  it('refuses damaged Opus files by name without ever trapping, as the release build runs them (#609)', () => {
    // xorshift, so that a failure replays: cuts, flipped bits, and runs of random bytes in the headers' region.
    let state = 0x2545f491
    const rand = (n: number) => {
      state ^= state << 13
      state ^= state >>> 17
      state ^= state << 5
      return (state >>> 0) % Math.max(1, n)
    }
    const module = new ImportModule(compiled)
    let refused = 0
    for (const name of ['sine.opus', 'sine-mono.opus', 'sine.webm', 'recorded.webm']) {
      const whole = fixture(name)
      for (let round = 0; round < 150; round++) {
        let bytes = whole.slice()
        if (round % 3 === 0) bytes = bytes.subarray(0, rand(bytes.length))
        else if (round % 3 === 1) for (let k = 0; k <= round % 16; k++) bytes[rand(bytes.length)]! ^= 1 << rand(8)
        else for (let at = rand(Math.min(bytes.length, 600)), k = 0; k < 1 + rand(24); k++) bytes[at + k] = rand(256)
        try {
          module.open(readerOf(bytes), bytes.length, name.split('.').pop()!)
          while (module.next());
          module.finish()
        } catch (error) {
          expect(error, `${name} round ${round}`).toBeInstanceOf(ImportError)
          refused++
        }
        // A trap would have broken the instance (abi.ts): every damaged file is a typed refusal or a decode.
        expect(module.broken, `${name} round ${round}`).toBe(false)
      }
    }
    expect(refused).toBeGreaterThan(0)
    // An Opus stream whose channels have a layout Faulk does not model.
    expect(() => module.open(readerOf(ambisonic()), ambisonic().length, 'webm')).toThrow(expect.objectContaining({ code: 'unsupported_codec' }) as Error)
  })

  it('caps the decoded size at the limit the page shows, 16 GiB (#780)', () => {
    const module = new ImportModule(compiled)
    expect(module.maxDecoded()).toBe(MAX_DECODED_BYTES)
    expect(MAX_DECODED_BYTES).toBe(16 * 2 ** 30)
  })

  it('refuses a damaged file by name, and the same instance imports the next one', () => {
    const module = new ImportModule(compiled)
    const wav = fixture('sine-s16.wav')
    const cut = wav.subarray(0, wav.length / 2 + 1)
    expect(() => {
      module.open(readerOf(cut), cut.length, 'wav')
      while (module.next());
      module.finish()
    }).toThrow(expect.objectContaining({ code: 'corrupt' }) as Error)
    module.open(readerOf(wav), wav.length, 'wav')
    while (module.next());
    expect(module.finish().frames).toBe(22_050)
  })
})

describe('the import pipeline over a project folder', () => {
  const file = (name: string, bytes = fixture(name)) => new File([bytes as Uint8Array<ArrayBuffer>], name)

  it('keeps the original bytes once under their SHA-256, and reads a WAV without a cache', async () => {
    const dir = new MemoryDir('song')
    const seen: number[] = []
    const first = await importAudio(dir.handle(), file('sine-s16.wav'), deps({ onProgress: (f) => seen.push(f) }))
    const hash = expected['sine-s16.wav']!.sha256!
    expect(first.media).toEqual({ hash, sample_rate: 44_100, channels: 2, frames: 22_050, name: 'sine-s16.wav' })
    expect(first.direct).toBe(true)
    expect(first.blob).toMatchObject({ path: `blobs/${hash}`, created: true })
    const blob = await (await (await dir.getDirectoryHandle('blobs')).getFileHandle(hash)).getFile()
    expect(sha256(new Uint8Array(await blob.arrayBuffer()))).toBe(hash)
    expect(seen.at(-1)).toBe(1)
    // Twice, under another name: one blob, still no cache, nothing left in `.faulk/tmp/`.
    const second = await importAudio(dir.handle(), file('copy.wav', fixture('sine-s16.wav')), deps())
    expect(second.media.hash).toBe(hash)
    expect(second.blob.created).toBe(false)
    expect(dir.paths().filter((p) => p.startsWith('blobs/'))).toEqual([`blobs/${hash}`])
    // The waveform peaks are its only cache.
    expect(dir.paths().filter((p) => p.startsWith('.faulk/'))).toEqual([peaksPath(hash)])
    expect(first.peaks).toBe(true)
  })

  it('writes the decoded PCM of a compressed file to its cache, which is valid and holds the decode', async () => {
    const dir = new MemoryDir('song')
    const { media, direct } = await importAudio(dir.handle(), file('sine.ogg'), deps())
    expect(direct).toBe(false)
    const cache = (await (await (await dir.getDirectoryHandle('.faulk')).getDirectoryHandle('cache')).getDirectoryHandle('pcm')).entries.get(
      `${media.hash}.pcm`,
    )
    const bytes = (cache as unknown as { bytes: Uint8Array }).bytes
    expect(bytes.length).toBe(CACHE_HEADER_BYTES + media.frames * media.channels * 4)
    expect(new ImportModule(compiled).cacheValid(bytes.subarray(0, CACHE_HEADER_BYTES), bytes.length, media)).toBe(true)
    expect(sha256(bytes.subarray(CACHE_HEADER_BYTES))).toBe(expected['sine.ogg']!.pcm_sha256)
    expect(dir.paths().sort()).toEqual([`.faulk/cache/pcm/${media.hash}.pcm`, peaksPath(media.hash), `blobs/${media.hash}`])
    expect(sha256(dir.bytes(peaksPath(media.hash))!)).toBe(expected['sine.ogg']!.peaks_sha256)
  })

  it('stores nothing for a file it refuses, and nothing either when cancelled', async () => {
    for (const refused of [file('not-audio.txt'), file('ambisonic.webm', ambisonic())]) {
      const dir = new MemoryDir('song')
      await expect(importAudio(dir.handle(), refused, deps())).rejects.toBeInstanceOf(ImportError)
      expect(dir.paths()).toEqual([])
    }
    const dir = new MemoryDir('song')
    const abort = new AbortController()
    const cancelled = importAudio(dir.handle(), file('sine.flac'), deps({ signal: abort.signal, onProgress: () => abort.abort() }))
    await expect(cancelled).rejects.toMatchObject({ code: 'cancelled' })
    expect(dir.paths()).toEqual([])
  })

  it('stores nothing for a file that decodes past the cap, refused at its header or by its blocks (#780)', async () => {
    // `sine.mp3` declares no length and decodes to 176 400 bytes: refused while its cache is being written.
    // `sine.flac` declares its length: refused at the opening, before any cache is made.
    for (const name of ['sine.mp3', 'sine.flac']) {
      const dir = new MemoryDir('song')
      const capped = deps({ module: async () => new ImportModule(compiled, 100_000) })
      await expect(importAudio(dir.handle(), file(name), capped)).rejects.toMatchObject({
        code: 'too_large',
        message: 'the file decodes to more than 100000 bytes of audio, the most a project keeps of one file',
      })
      expect(dir.paths()).toEqual([])
    }
    // Under the cap, the same instance setting imports.
    const dir = new MemoryDir('song')
    const outcome = await importAudio(dir.handle(), file('sine.mp3'), deps({ module: async () => new ImportModule(compiled, 200_000) }))
    expect(dir.paths()).toContain(cachePath(outcome.media.hash))
  })

  it('stores nothing when the folder refuses a write, and says so as the folder did', async () => {
    for (const failing of ['import-', '.pcm']) {
      const dir = new MemoryDir('song')
      dir.hooks.failWrite = (name) => name.startsWith(failing) || name.endsWith(failing)
      const refused = importAudio(dir.handle(), file('sine.mp3'), deps())
      await expect(refused).rejects.toMatchObject({ name: 'QuotaExceededError' })
      expect(dir.paths()).toEqual([])
    }
    // The peaks are a cache: a write of them refused leaves the import whole, without them.
    const dir = new MemoryDir('song')
    dir.hooks.failWrite = (name) => name.endsWith('.peaks')
    const outcome = await importAudio(dir.handle(), file('sine.mp3'), deps())
    expect(outcome.peaks).toBe(false)
    expect(dir.paths().sort()).toEqual([cachePath(outcome.media.hash), `blobs/${outcome.media.hash}`])
  })

  it('makes the blob in place where the folder cannot move a file, and the cache too', async () => {
    const dir = new MemoryDir('song')
    dir.hooks.noMove = true
    const { media } = await importAudio(dir.handle(), file('sine.ogg'), deps())
    expect(dir.paths().sort()).toEqual([`.faulk/cache/pcm/${media.hash}.pcm`, peaksPath(media.hash), `blobs/${media.hash}`])
    // A write that fails in place removes the name: no half-written blob under a hash.
    const other = new MemoryDir('song')
    other.hooks.noMove = true
    other.hooks.failClose = (name) => name === expected['sine.flac']!.sha256
    await expect(importAudio(other.handle(), file('sine.flac'), deps())).rejects.toBeDefined()
    expect(other.paths().filter((p) => p.startsWith('blobs/'))).toEqual([])
  })

  it('two imports of the same file at once make one blob', async () => {
    const dir = new MemoryDir('song')
    const [a, b] = await Promise.all([
      importAudio(dir.handle(), file('sine.flac'), deps()),
      importAudio(dir.handle(), file('b.flac', fixture('sine.flac')), deps()),
    ])
    expect(a.media.hash).toBe(b.media.hash)
    expect([a.blob.created, b.blob.created].sort()).toEqual([false, true])
    expect(dir.paths().filter((p) => p.startsWith('blobs/')).length).toBe(1)
  })

  it('an import and the caches of an opening run at once in one Worker, their steps interleaved, each with its own instance', async () => {
    const one = deps()
    const dir = new MemoryDir('song')
    const other = new MemoryDir('other')
    const m4a = expected['sine.m4a']!
    other.put(`blobs/${m4a.sha256}`, fixture('sine.m4a'))
    const [imported, report] = await Promise.all([
      importAudio(dir.handle(), file('sine.mp3'), one),
      ensureAudioCache(other.handle(), [{ hash: m4a.sha256!, sample_rate: 44_100, channels: 2, frames: 22_050 }], one),
    ])
    expect(imported.media.frames).toBe(22_050)
    expect(report).toEqual({ made: [m4a.sha256], peaks: [m4a.sha256], missing: [], failed: [] })
    const cache = (other.entries.get('.faulk') as MemoryDir).entries.get('cache') as MemoryDir
    const made = (cache.entries.get('pcm') as MemoryDir).entries.get(`${m4a.sha256}.pcm`) as unknown as { bytes: Uint8Array }
    const native = new Float32Array(new Uint8Array(readFileSync(new URL('sine.m4a.pcm', FIXTURES))).buffer)
    const page = new Float32Array(made.bytes.slice(CACHE_HEADER_BYTES).buffer)
    expect(page.reduce((w, s, i) => Math.max(w, Math.abs(s - native[i]!)), 0)).toBeLessThanOrEqual(1e-6)
  })

  it('two imports of the same compressed file at once: one blob and one whole cache, no failure', async () => {
    const dir = new MemoryDir('song')
    const outcomes = await Promise.all([
      importAudio(dir.handle(), file('sine.mp3'), deps()),
      importAudio(dir.handle(), file('again.mp3', fixture('sine.mp3')), deps()),
    ])
    expect(outcomes[0]!.media.hash).toBe(outcomes[1]!.media.hash)
    const hash = outcomes[0]!.media.hash
    expect(dir.paths().sort()).toEqual([`.faulk/cache/pcm/${hash}.pcm`, peaksPath(hash), `blobs/${hash}`])
    expect(sha256(dir.bytes(peaksPath(hash))!)).toBe(expected['sine.mp3']!.peaks_sha256)
  })

  it('names a media as the project takes it: trimmed, 200 characters at most', () => {
    expect(mediaName('  kick.wav ')).toBe('kick.wav')
    expect(Array.from(mediaName(`${'é'.repeat(300)}.wav`)).length).toBe(200)
  })
})

describe('the PCM caches an opening makes again', () => {
  const ref = (name: string) => {
    const r = expected[name]!
    return { hash: r.sha256!, sample_rate: r.sample_rate!, channels: r.channels!, frames: r.frames! }
  }

  it('makes a missing or stale cache again, leaves a valid one and a direct file alone, and reports a missing blob', async () => {
    const dir = new MemoryDir('song')
    dir.put(`blobs/${ref('sine.mp3').hash}`, fixture('sine.mp3'))
    dir.put(`blobs/${ref('sine.m4a').hash}`, fixture('sine.m4a'))
    dir.put(`blobs/${ref('sine-s24.wav').hash}`, fixture('sine-s24.wav'))
    // The AAC file's cache is there but cut short.
    dir.put(cachePath(ref('sine.m4a').hash), new Uint8Array(100))
    const media = [ref('sine.mp3'), ref('sine.m4a'), ref('sine-s24.wav'), ref('sine.ogg'), ref('sine.mp3')]
    const report = await ensureAudioCache(dir.handle(), media, deps())
    expect(report).toEqual({
      made: [ref('sine.mp3').hash, ref('sine.m4a').hash],
      // The WAV is read directly (no PCM cache) but still gets its peaks.
      peaks: [ref('sine.mp3').hash, ref('sine.m4a').hash, ref('sine-s24.wav').hash],
      missing: [ref('sine.ogg').hash],
      failed: [],
    })
    for (const name of ['sine.mp3', 'sine.m4a']) {
      const bytes = new TextEncoder().encode(dir.text(cachePath(ref(name).hash)) ?? '')
      expect(bytes.length).toBeGreaterThan(0)
    }
    // Once made, nothing to do.
    expect(await ensureAudioCache(dir.handle(), media, deps())).toEqual({ made: [], peaks: [], missing: [ref('sine.ogg').hash], failed: [] })
  })

  it('makes the peaks a project imported before them lacks, leaving a valid PCM cache alone, and remakes doubtful ones', async () => {
    const dir = new MemoryDir('song')
    const mp3 = await importAudio(dir.handle(), new File([fixture('sine.mp3') as Uint8Array<ArrayBuffer>], 'a.mp3'), deps())
    const wav = await importAudio(dir.handle(), new File([fixture('sine-s16.wav') as Uint8Array<ArrayBuffer>], 'b.wav'), deps())
    const media = [mp3.media, wav.media]
    const pcm = dir.bytes(cachePath(mp3.media.hash))!
    const good = dir.bytes(peaksPath(mp3.media.hash))!
    // As before AU-2: no peaks at all.
    dir.remove(peaksPath(mp3.media.hash))
    dir.remove(peaksPath(wav.media.hash))
    expect(await ensureAudioCache(dir.handle(), media, deps())).toEqual({ made: [], peaks: [mp3.media.hash, wav.media.hash], missing: [], failed: [] })
    expect(dir.bytes(peaksPath(mp3.media.hash))).toEqual(good)
    expect(dir.bytes(cachePath(mp3.media.hash))).toEqual(pcm)
    // Of another decoder (byte 24), cut short, or longer: made again.
    for (const doubtful of [Uint8Array.from(good, (b, i) => (i === 24 ? b + 1 : b)), good.slice(0, good.length - 1), Uint8Array.from([...good, 0])]) {
      dir.put(peaksPath(mp3.media.hash), doubtful)
      expect((await ensureAudioCache(dir.handle(), media, deps())).peaks).toEqual([mp3.media.hash])
      expect(dir.bytes(peaksPath(mp3.media.hash))).toEqual(good)
    }
    expect((await ensureAudioCache(dir.handle(), media, deps())).peaks).toEqual([])
  })

  it('a peaks write the folder refuses leaves the PCM cache made and the media not failed', async () => {
    const dir = new MemoryDir('song')
    const m = ref('sine.mp3')
    dir.put(`blobs/${m.hash}`, fixture('sine.mp3'))
    dir.hooks.failWrite = (name) => name.endsWith('.peaks')
    expect(await ensureAudioCache(dir.handle(), [m], deps())).toEqual({ made: [m.hash], peaks: [], missing: [], failed: [] })
    expect(dir.paths().sort()).toEqual([cachePath(m.hash), `blobs/${m.hash}`])
  })

  it('reports a blob that decodes past the cap and keeps no cache of it (#780)', async () => {
    const dir = new MemoryDir('song')
    const m = ref('sine.mp3')
    dir.put(`blobs/${m.hash}`, fixture('sine.mp3'))
    const report = await ensureAudioCache(dir.handle(), [m], deps({ module: async () => new ImportModule(compiled, 100_000) }))
    expect(report.failed).toEqual([{ hash: m.hash, code: 'too_large' }])
    expect(report.made).toEqual([])
    expect(dir.paths()).toEqual([`blobs/${m.hash}`])
  })

  it('reports a blob that does not decode as the project says, and keeps no cache of it', async () => {
    const dir = new MemoryDir('song')
    const wrong = { ...ref('sine.mp3'), frames: 1000 }
    dir.put(`blobs/${wrong.hash}`, fixture('sine.mp3'))
    const report = await ensureAudioCache(dir.handle(), [wrong], deps())
    expect(report.failed).toEqual([{ hash: wrong.hash, code: 'corrupt' }])
    expect(dir.paths()).toEqual([`blobs/${wrong.hash}`])
  })
})

describe('the store and a running import', () => {
  it('the clean-up of an attach leaves the temporary files of an import that runs, and removes those of one that ended', async () => {
    const dir = new MemoryDir('song')
    const running = 'run-1'
    const dead = 'dead-1'
    dir.put(`.faulk/tmp/${IMPORT_TMP_PREFIX}${running}`, 'copy')
    dir.put(`.faulk/tmp/${IMPORT_TMP_PREFIX}${running}.pcm`, 'cache')
    dir.put(`.faulk/tmp/${IMPORT_TMP_PREFIX}${dead}`, 'copy')
    dir.put('.faulk/tmp/other', 'x')
    const store = new FolderBackend(dir.handle(), () => deps())
    let release = () => {}
    const held = new Promise<void>((resolve) => (release = resolve))
    const lock = navigator.locks.request(importLock(running), () => held)
    await store.cleanUp()
    expect(dir.paths().sort()).toEqual([`.faulk/tmp/${IMPORT_TMP_PREFIX}${running}`, `.faulk/tmp/${IMPORT_TMP_PREFIX}${running}.pcm`])
    release()
    await lock
    await store.cleanUp()
    expect(dir.paths()).toEqual([])
  })

  it('a folder store imports through the pipeline, and refuses where no Worker can read a file', async () => {
    const dir = new MemoryDir('song')
    const store = new FolderBackend(dir.handle(), (onProgress, signal) => deps({ onProgress, signal }))
    const outcome = await store.importAudio(new File([fixture('sine-s16.aiff') as Uint8Array<ArrayBuffer>], 'a.aiff'))
    expect(outcome.direct).toBe(true)
    const page = new FolderBackend(dir.handle())
    await expect(page.importAudio(new File([fixture('sine-s16.aiff') as Uint8Array<ArrayBuffer>], 'a.aiff'))).rejects.toMatchObject({ code: 'unavailable' })
    expect(StoreError).toBeDefined()
  })
})
