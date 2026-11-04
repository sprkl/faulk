// Importing an audio file into a project folder (AU-1), and making again the PCM caches a
// folder lacks. Runs in the project store's Worker (`storage/worker.ts`), on the folder's root
// handle: the one place that writes the folder (décision 12). Everything streams: the file is
// read in slices (`ReadSync`, `FileReaderSync` in the Worker) and its decoded audio written as it
// comes, so a file of an hour never sits in memory. Between steps the Worker's event loop runs
// (its other calls, a cancel).
//
// An import: (1) hash the file while copying it to `.faulk/tmp/import-<id>`; (2) decode it (every
// packet, so a damaged file is refused now), writing the PCM cache to another temporary file when
// the file cannot be read directly; (3) only then put the copy in `blobs/<sha256>` (or drop it
// when the folder already has that blob: one blob per file, however often it is imported) and the
// cache in `.faulk/cache/pcm/`. A failure anywhere removes the temporary files and stores nothing.
import { CACHE_HEADER_BYTES, HEAD_BYTES, ImportError, type ImportModule, type MediaInfo, type ReadSync } from './abi'

/** A media as `clip.import_audio` takes it (`project::AudioMedia`). */
export interface ImportedMedia extends MediaInfo {
  /** The SHA-256 of the file, the name of its blob. */
  hash: string
  /** The file's name (trimmed, at most 200 characters, as the project takes it). */
  name: string
}

export interface ImportOutcome {
  media: ImportedMedia
  /** Read directly from the blob (WAV, AIFF of plain samples): no PCM cache. */
  direct: boolean
  /** The blob's path and stamp (`<size>:<modified>`, as a scan sees it), and whether this import wrote it. */
  blob: { path: string; stamp: string; created: boolean }
}

/** A media the project uses, as `ensureAudioCache` checks it. */
export type MediaRef = { hash: string } & MediaInfo

export interface CacheReport {
  /** Caches made (again). */
  made: string[]
  /** Media whose blob the folder lacks. */
  missing: string[]
  /** Media whose blob could not be decoded. */
  failed: { hash: string; code: string }[]
}

export interface PipelineDeps {
  /** A new instance of the import module, for one job: an instance holds one job's state, and jobs interleave. */
  module(): Promise<ImportModule>
  /** A synchronous reader of a file (`FileReaderSync` in a Worker). */
  readerOf(blob: Blob): Promise<ReadSync>
  /** Progress from 0 to 1. */
  onProgress?: (fraction: number) => void
  signal?: AbortSignal
}

/** The bytes read or written at once. */
const SLICE = 1 << 20
/** The longest the pipeline runs without giving the Worker's event loop a turn, in ms. */
const SLICE_MS = 25
/** Progress is reported at most this often, in ms. */
const PROGRESS_MS = 100

export const CACHE_DIR = ['.faulk', 'cache', 'pcm']
export const cachePath = (hash: string) => `${CACHE_DIR.join('/')}/${hash}.pcm`

/** The prefix of an import's temporary files in `.faulk/tmp/` (the store's clean-up leaves those whose import runs). */
export const IMPORT_TMP_PREFIX = 'import-'

/** The Web Lock an import holds on its temporary files while it runs. */
export const importLock = (id: string) => `faulk.import.${id}`

const MAX_NAME = 200

/** The name a project keeps of a file: trimmed, at most 200 characters. */
export function mediaName(name: string): string {
  return Array.from(name.trim()).slice(0, MAX_NAME).join('').trim()
}

/** The file's extension, lower case (a hint to the decoder only). */
export function extensionOf(name: string): string {
  const at = name.lastIndexOf('.')
  const ext = at > 0 ? name.slice(at + 1).toLowerCase() : ''
  return /^[a-z0-9]{1,8}$/.test(ext) ? ext : ''
}

const stampOf = (file: File) => `${file.size}:${file.lastModified}`

async function directory(root: FileSystemDirectoryHandle, parts: string[], create: boolean): Promise<FileSystemDirectoryHandle> {
  let dir = root
  for (const part of parts) dir = await dir.getDirectoryHandle(part, { create })
  return dir
}

async function maybeFile(dir: FileSystemDirectoryHandle, name: string): Promise<FileSystemFileHandle | null> {
  try {
    return await dir.getFileHandle(name)
  } catch (error) {
    if (error instanceof DOMException && (error.name === 'NotFoundError' || error.name === 'TypeMismatchError')) return null
    throw error
  }
}

const turn = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

/** Throws when the import was cancelled. */
function checkCancel(signal?: AbortSignal) {
  if (signal?.aborted) throw new ImportError('cancelled', 'the import was cancelled')
}

/** Reports progress at most every `PROGRESS_MS`, and the end always. */
function progressReporter(onProgress?: (fraction: number) => void) {
  let last = -Infinity
  return (fraction: number, force = false) => {
    if (!onProgress) return
    const now = performance.now()
    if (!force && now - last < PROGRESS_MS) return
    last = now
    onProgress(Math.max(0, Math.min(1, fraction)))
  }
}

/** A temporary file of `.faulk/tmp/`, written in a stream and then moved into place or removed. */
class TempFile {
  private writable: FileSystemWritableFileStream | null = null
  private constructor(
    readonly dir: FileSystemDirectoryHandle,
    readonly name: string,
    readonly handle: FileSystemFileHandle,
  ) {}

  static async create(root: FileSystemDirectoryHandle, name: string): Promise<TempFile> {
    const dir = await directory(root, ['.faulk', 'tmp'], true)
    const handle = await dir.getFileHandle(name, { create: true })
    const temp = new TempFile(dir, name, handle)
    temp.writable = await handle.createWritable()
    return temp
  }

  async write(bytes: Uint8Array) {
    await this.writable!.write(bytes as Uint8Array<ArrayBuffer>)
  }

  /** Writes over bytes already written, at `position` (the cache's header, once its length is known). */
  async writeAt(position: number, bytes: Uint8Array) {
    await this.writable!.write({ type: 'write', position, data: bytes as Uint8Array<ArrayBuffer> })
  }

  async close() {
    const writable = this.writable
    this.writable = null
    await writable?.close()
  }

  /** Moves the closed file to `dir/name`, replacing nothing: false when `move` is not available here. */
  async moveTo(dir: FileSystemDirectoryHandle, name: string): Promise<boolean> {
    const move = (this.handle as unknown as { move?: (to: FileSystemDirectoryHandle, name: string) => Promise<void> }).move
    if (typeof move !== 'function') return false
    try {
      await move.call(this.handle, dir, name)
      return true
    } catch {
      return false
    }
  }

  /** Removes the file (best effort), whatever state it is in. */
  async discard() {
    await this.writable?.abort().catch(() => undefined)
    this.writable = null
    await this.dir.removeEntry(this.name).catch(() => undefined)
  }
}

/** Copies a temporary file's bytes into `dir/name` with `createWritable` (where `move` is refused). The name is removed again when the copy fails. */
async function copyInto(dir: FileSystemDirectoryHandle, name: string, from: FileSystemFileHandle) {
  const handle = await dir.getFileHandle(name, { create: true })
  const writable = await handle.createWritable()
  try {
    const reader = (await from.getFile()).stream().getReader()
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      await writable.write(value as Uint8Array<ArrayBuffer>)
    }
    await writable.close()
  } catch (error) {
    await writable.abort().catch(() => undefined)
    await dir.removeEntry(name).catch(() => undefined)
    throw error
  }
}

/**
 * Decodes the file `source` reads, every packet, writing the PCM cache into `cache` when given.
 * Returns the metadata. Progress goes from `from` to `to` as the decoder reads the file.
 */
async function decode(
  module: ImportModule,
  read: ReadSync,
  size: number,
  extension: string,
  cache: TempFile | null,
  deps: PipelineDeps,
  progress: (fraction: number) => void,
  span: [number, number],
): Promise<MediaInfo> {
  module.open(read, size, extension)
  const buffer = new Uint8Array(SLICE)
  let filled = 0
  let header = false
  let since = performance.now()
  const flush = async () => {
    if (!cache || filled === 0) return
    await cache.write(buffer.slice(0, filled))
    filled = 0
  }
  for (;;) {
    checkCancel(deps.signal)
    const block = module.next()
    if (!block) break
    if (cache) {
      if (!header) {
        // The header comes first, but its frame count is known only at the end: written then, see below.
        filled = CACHE_HEADER_BYTES
        header = true
      }
      const bytes = new Uint8Array(block.buffer, block.byteOffset, block.byteLength)
      let at = 0
      while (at < bytes.length) {
        const n = Math.min(bytes.length - at, SLICE - filled)
        buffer.set(bytes.subarray(at, at + n), filled)
        filled += n
        at += n
        if (filled === SLICE) await flush()
      }
    }
    progress(span[0] + (span[1] - span[0]) * (size > 0 ? module.position() / size : 1))
    if (performance.now() - since > SLICE_MS) {
      await turn()
      since = performance.now()
    }
  }
  const info = module.finish()
  if (cache) {
    await flush()
    // The header, now that the length is known: written over the 32 bytes kept for it.
    await cache.writeAt(0, module.cacheHeader(info))
  }
  return info
}

/** The first bytes of a file. */
function head(read: ReadSync, size: number): Uint8Array {
  return read(0, Math.min(size, HEAD_BYTES))
}

/** Puts a decoded cache in place (replacing a stale one). */
async function placeCache(root: FileSystemDirectoryHandle, hash: string, temp: TempFile) {
  // One job at a time per media in this Worker (two imports of one file, an import and an opening's caches).
  await serialised(`${hash}.pcm`, async () => {
    const dir = await directory(root, CACHE_DIR, true)
    const name = `${hash}.pcm`
    await dir.removeEntry(name).catch((error: unknown) => {
      if (!(error instanceof DOMException && error.name === 'NotFoundError')) throw error
    })
    if (!(await temp.moveTo(dir, name))) {
      // A cache made in place and cut short is harmless: its size says so, and it is made again.
      await copyInto(dir, name, temp.handle)
      await temp.discard()
    }
  })
}

/** The imports of a blob in this Worker, one at a time: two imports of the same file never race on its name. */
const placing = new Map<string, Promise<unknown>>()

async function serialised<T>(hash: string, task: () => Promise<T>): Promise<T> {
  const before = placing.get(hash) ?? Promise.resolve()
  const run = before.then(task, task)
  const tail = run.catch(() => undefined)
  placing.set(hash, tail)
  try {
    return await run
  } finally {
    // The last one out forgets the hash; a later import of it chained itself already.
    if (placing.get(hash) === tail) placing.delete(hash)
  }
}

/** Runs `task` holding the Web Lock of an import's temporary files, where Web Locks exist. */
async function holdingLock<T>(id: string, task: () => Promise<T>): Promise<T> {
  const locks = (globalThis.navigator as Navigator | undefined)?.locks
  if (!locks) return task()
  return locks.request(importLock(id), task) as Promise<T>
}

/** Imports `file` into the project folder at `root`. Throws an `ImportError`. */
export async function importAudio(root: FileSystemDirectoryHandle, file: File, deps: PipelineDeps): Promise<ImportOutcome> {
  const id = crypto.randomUUID()
  return holdingLock(id, () => runImport(root, file, deps, id))
}

async function runImport(root: FileSystemDirectoryHandle, file: File, deps: PipelineDeps, id: string): Promise<ImportOutcome> {
  const progress = progressReporter(deps.onProgress)
  const size = file.size
  const read = await deps.readerOf(file)
  let copy: TempFile | null = null
  let cache: TempFile | null = null
  try {
    // (1) The hash, and a copy of the bytes it was computed on.
    const module = await deps.module()
    copy = await TempFile.create(root, `${IMPORT_TMP_PREFIX}${id}`)
    module.hashBegin()
    for (let at = 0; at < size; at += SLICE) {
      checkCancel(deps.signal)
      const bytes = read(at, Math.min(SLICE, size - at))
      if (bytes.length === 0) throw new ImportError('io', 'the file ended before its size')
      module.hashPush(bytes)
      await copy.write(bytes)
      progress((0.5 * (at + bytes.length)) / Math.max(1, size))
    }
    const hash = module.hashEnd()
    await copy.close()
    // (2) Decoding, from the user's file (the copy is not readable before it is moved).
    const start = head(read, size)
    // A file that looks read directly is decoded without a cache, and checked afterwards; in the
    // rare case the check fails (the headers and the decoder disagree), it is decoded again into one.
    const maybeDirect = module.maybeDirect(start, size)
    if (!maybeDirect) cache = await TempFile.create(root, `${IMPORT_TMP_PREFIX}${id}.pcm`)
    const info = await decode(module, read, size, extensionOf(file.name), cache, deps, progress, [0.5, 1])
    const direct = maybeDirect && module.direct(start, size, info)
    if (!maybeDirect) {
      await cache!.close()
    } else if (!direct) {
      cache = await TempFile.create(root, `${IMPORT_TMP_PREFIX}${id}.pcm`)
      await decode(await deps.module(), read, size, extensionOf(file.name), cache, deps, () => undefined, [1, 1])
      await cache.close()
    }
    checkCancel(deps.signal)
    // (3) Into place.
    const blob = await serialised(hash, async () => {
      const blobs = await directory(root, ['blobs'], true)
      let found = await maybeFile(blobs, hash)
      const created = !found
      if (!found) {
        if (!(await copy!.moveTo(blobs, hash))) {
          await copyInto(blobs, hash, copy!.handle)
        }
        found = await blobs.getFileHandle(hash)
      }
      return { path: `blobs/${hash}`, stamp: stampOf(await found.getFile()), created }
    })
    await copy.discard()
    copy = null
    if (cache) await placeCache(root, hash, cache)

    cache = null
    progress(1, true)
    return { media: { hash, ...info, name: mediaName(file.name) }, direct, blob }
  } catch (error) {
    await copy?.discard()
    await cache?.discard()
    if (error instanceof ImportError) throw error
    const name = error instanceof DOMException ? error.name : ''
    if (name === 'NotReadableError' || name === 'NotFoundError') throw new ImportError('io', `the file could not be read (${name})`)
    throw error
  }
}

/**
 * Makes again the PCM caches the folder lacks (the cache absent regenerates on opening, in the
 * target that holds the disk: the page, since the helper holds no disk). A cache whole and of the
 * media is left alone; a media read directly needs none; a missing blob is reported, not an error.
 */
export async function ensureAudioCache(root: FileSystemDirectoryHandle, media: MediaRef[], deps: PipelineDeps): Promise<CacheReport> {
  const report: CacheReport = { made: [], missing: [], failed: [] }
  const progress = progressReporter(deps.onProgress)
  const seen = new Set<string>()
  const list = media.filter((m) => !seen.has(m.hash) && seen.add(m.hash))
  for (const [index, m] of list.entries()) {
    checkCancel(deps.signal)
    const span: [number, number] = [index / list.length, (index + 1) / list.length]
    const blobs = await directory(root, ['blobs'], false).catch(() => null)
    const blobHandle = blobs && (await maybeFile(blobs, m.hash))
    if (!blobHandle) {
      report.missing.push(m.hash)
      continue
    }
    const module = await deps.module()
    const cacheDir = await directory(root, CACHE_DIR, false).catch(() => null)
    const cached = cacheDir && (await maybeFile(cacheDir, `${m.hash}.pcm`))
    if (cached) {
      const file = await cached.getFile()
      const start = new Uint8Array(await file.slice(0, CACHE_HEADER_BYTES).arrayBuffer())
      if (module.cacheValid(start, file.size, m)) continue
    }
    const blob = await blobHandle.getFile()
    const read = await deps.readerOf(blob)
    if (module.direct(head(read, blob.size), blob.size, m)) continue
    const id = crypto.randomUUID()
    await holdingLock(id, async () => {
      const temp = await TempFile.create(root, `${IMPORT_TMP_PREFIX}${id}.pcm`)
      try {
        const info = await decode(await deps.module(), read, blob.size, '', temp, deps, progress, span)
        await temp.close()
        if (info.frames !== m.frames || info.channels !== m.channels || info.sample_rate !== m.sample_rate) {
          throw new ImportError('corrupt', 'the file decodes differently from what the project says of it')
        }
        await placeCache(root, m.hash, temp)
        report.made.push(m.hash)
      } catch (error) {
        await temp.discard()
        if (error instanceof ImportError && error.code === 'cancelled') throw error
        report.failed.push({ hash: m.hash, code: error instanceof ImportError ? error.code : 'io' })
      }
    })
  }
  progress(1, true)
  return report
}
