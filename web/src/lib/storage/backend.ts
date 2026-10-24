// The project folder over the File System Access API (SV-2): what the Worker runs, and what the
// tests run against a stand-in directory. Plain functions on a `FileSystemDirectoryHandle`; no
// state beyond the handle, so a call that fails leaves nothing to repair.
import { needsBytes, type FolderEntry, type SaveEntry } from '../project/core'
import {
  IGNORED_FILES,
  SKIPPED_DIRS,
  StoreError,
  SWAP_SUFFIX,
  type ProjectStore,
  type Signature,
  type StoreErrorCode,
} from './types'

/** `FileSystemDirectoryHandle.values()`, which `lib.dom` does not type everywhere. */
type Listable = { values(): AsyncIterable<FileSystemHandle> }
const children = (dir: FileSystemDirectoryHandle): AsyncIterable<FileSystemHandle> => (dir as unknown as Listable).values()

/** Reads this many files at once: thousands of small files are the cost of a folder, not their size. */
const CONCURRENCY = 16

/** What a failed call of the file system means to the page. */
export function classify(error: unknown): StoreError {
  if (error instanceof StoreError) return error
  const name = error instanceof DOMException ? error.name : error instanceof Error ? error.name : ''
  const message = error instanceof Error ? error.message : String(error)
  let code: StoreErrorCode = 'io'
  if (name === 'NotAllowedError' || name === 'SecurityError') code = 'permission'
  else if (name === 'QuotaExceededError') code = 'full'
  else if (name === 'NotFoundError') code = 'missing'
  return new StoreError(code, `${name || 'Error'}: ${message}`)
}

let tmpCounter = 0

const stamp = (file: File) => `${file.size}:${file.lastModified}`

/** The directory at `parts` below `root`, made on the way when `create`. */
async function directory(root: FileSystemDirectoryHandle, parts: string[], create: boolean): Promise<FileSystemDirectoryHandle> {
  let dir = root
  for (const part of parts) dir = await dir.getDirectoryHandle(part, { create })
  return dir
}

/** What of the root belongs to the project: `project.json`, the tracks and the blobs. A README, a mixdown, the `.DS_Store` the file manager writes are the user's, never a change to the project. */
const PROJECT_ROOT_FILES: readonly string[] = ['project.json']
const PROJECT_ROOT_DIRS: readonly string[] = ['tracks', 'blobs']

/** Every file of the project below `dir` that a scan looks at: [path, handle], skipping `.git`, `.faulk`, swap files and what is not the project's. */
async function* walk(dir: FileSystemDirectoryHandle, base = ''): AsyncGenerator<[string, FileSystemFileHandle]> {
  for await (const entry of children(dir)) {
    const path = base ? `${base}/${entry.name}` : entry.name
    if (entry.kind === 'directory') {
      if (SKIPPED_DIRS.includes(entry.name) || (base === '' && !PROJECT_ROOT_DIRS.includes(entry.name))) continue
      yield* walk(entry as FileSystemDirectoryHandle, path)
    } else if (!entry.name.endsWith(SWAP_SUFFIX) && (base !== '' || PROJECT_ROOT_FILES.includes(entry.name))) {
      yield [path, entry as FileSystemFileHandle]
    }
  }
}

/** Runs `task` on each item, at most `CONCURRENCY` at a time; the first failure rejects. */
async function each<T>(items: T[], task: (item: T) => Promise<void>) {
  let next = 0
  const worker = async () => {
    while (next < items.length) await task(items[next++])
  }
  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, items.length) }, worker))
}

export class FolderBackend implements ProjectStore {
  constructor(readonly root: FileSystemDirectoryHandle) {}

  get name() {
    return this.root.name
  }

  async isEmpty(): Promise<boolean> {
    try {
      for await (const entry of children(this.root)) {
        if (entry.kind === 'file' && IGNORED_FILES.includes(entry.name)) continue
        return false
      }
      return true
    } catch (error) {
      throw classify(error)
    }
  }

  async hasProject(): Promise<boolean> {
    try {
      await this.root.getFileHandle('project.json')
      return true
    } catch (error) {
      if (error instanceof DOMException && (error.name === 'NotFoundError' || error.name === 'TypeMismatchError')) return false
      throw classify(error)
    }
  }

  async readFolder(): Promise<{ entries: FolderEntry[]; signature: Signature }> {
    try {
      const files: [string, FileSystemFileHandle][] = []
      for await (const found of walk(this.root)) files.push(found)
      // The session file is read; the rest of `.faulk/` (cache, temporary files) is the folder's own business.
      // The session is read, and so are the user's `.gitignore` and `.gitattributes`: a save leaves what exists alone.
      for (const extra of ['.faulk/session.json', '.gitignore', '.gitattributes']) {
        const found = await this.maybeFile(extra)
        if (found) files.push(found)
      }
      const entries: FolderEntry[] = new Array(files.length)
      const signature: Signature = {}
      await each(
        files.map((file, index) => ({ file, index })),
        async ({ file: [path, handle], index }) => {
          const file = await handle.getFile()
          if (!path.startsWith('.faulk/') && !path.startsWith('.git')) signature[path] = stamp(file)
          if (!path.startsWith('blobs/')) {
            entries[index] = { path, bytes: new Uint8Array(await file.arrayBuffer()) }
            return
          }
          const head = new Uint8Array(await file.slice(0, 8).arrayBuffer())
          entries[index] = needsBytes(path, head) ? { path, bytes: new Uint8Array(await file.arrayBuffer()) } : { path, size: file.size }
        },
      )
      return { entries, signature }
    } catch (error) {
      throw classify(error)
    }
  }

  private async maybeFile(path: string): Promise<[string, FileSystemFileHandle] | null> {
    const parts = path.split('/')
    try {
      const dir = await directory(this.root, parts.slice(0, -1), false)
      return [path, await dir.getFileHandle(parts[parts.length - 1])]
    } catch (error) {
      if (error instanceof DOMException && (error.name === 'NotFoundError' || error.name === 'TypeMismatchError')) return null
      throw error
    }
  }

  async read(path: string, start = 0, end?: number): Promise<Uint8Array> {
    try {
      const parts = path.split('/')
      const dir = await directory(this.root, parts.slice(0, -1), false)
      const file = await (await dir.getFileHandle(parts[parts.length - 1])).getFile()
      return new Uint8Array(await file.slice(start, end).arrayBuffer())
    } catch (error) {
      throw classify(error)
    }
  }

  async apply(entries: SaveEntry[]): Promise<Signature> {
    const written: Signature = {}
    try {
      for (const entry of entries) {
        const parts = entry.path.split('/')
        if (entry.type === 'remove') {
          await this.remove(parts)
          continue
        }
        const dir = await directory(this.root, parts.slice(0, -1), true)
        const name = parts[parts.length - 1]
        const existing = await dir.getFileHandle(name).catch((error: unknown) => {
          if (error instanceof DOMException && error.name === 'NotFoundError') return null
          throw error
        })
        const handle = existing ? await this.replace(existing, entry.bytes) : await this.create(dir, name, entry.bytes)
        if (!entry.path.startsWith('.faulk/')) written[entry.path] = stamp(await handle.getFile())
      }
      return written
    } catch (error) {
      throw classify(error)
    }
  }

  /**
   * Replaces a file's content. `createWritable` writes a swap file and replaces the target when it
   * is closed: a tab closed or a crash before `close` leaves the old file whole (File System Access
   * spec), and an error leaves it whole too.
   */
  private async replace(handle: FileSystemFileHandle, bytes: Uint8Array): Promise<FileSystemFileHandle> {
    const writable = await handle.createWritable()
    try {
      await writable.write(bytes as Uint8Array<ArrayBuffer>)
      await writable.close()
    } catch (error) {
      await writable.abort().catch(() => undefined)
      throw error
    }
    return handle
  }

  /** Whether `move` failed because this folder does not support it, found once and remembered. */
  private moveUnsupported = false

  /**
   * Makes a new file. A file made by name exists, empty, from that moment: a tab closed before it
   * is written leaves an empty `track.json`, and the reader refuses the project. So the content is
   * written to a file in `.faulk/tmp/` (atomic like any other) and moved into place: the new name
   * appears whole or not at all. Where `move` is not available (not every folder the browser gives
   * supports it) the file is made in place, and a write that fails then removes it again.
   */
  private async create(dir: FileSystemDirectoryHandle, name: string, bytes: Uint8Array): Promise<FileSystemFileHandle> {
    if (!this.moveUnsupported) {
      const tmpDir = await directory(this.root, ['.faulk', 'tmp'], true)
      const tmpName = `${Date.now().toString(36)}-${(tmpCounter++).toString(36)}`
      const tmp = await tmpDir.getFileHandle(tmpName, { create: true })
      try {
        await this.replace(tmp, bytes)
      } catch (error) {
        // The content could not be written (a full disk, a refused permission): that is the caller's to hear.
        await tmpDir.removeEntry(tmpName).catch(() => undefined)
        throw error
      }
      try {
        const move = (tmp as unknown as { move?: (to: FileSystemDirectoryHandle, name: string) => Promise<void> }).move
        if (typeof move !== 'function') throw new TypeError('move is not available')
        await move.call(tmp, dir, name)
        return tmp
      } catch {
        // Whatever the browser answers (not implemented, not allowed for a folder the user chose, a
        // refused rename), the file is made in place from now on: a failure there is reported as it is.
        await tmpDir.removeEntry(tmpName).catch(() => undefined)
        this.moveUnsupported = true
      }
    }
    const handle = await dir.getFileHandle(name, { create: true })
    try {
      return await this.replace(handle, bytes)
    } catch (error) {
      await dir.removeEntry(name).catch(() => undefined)
      throw error
    }
  }

  /** Removes a file and the directories it leaves empty (never the root). */
  private async remove(parts: string[]) {
    for (let depth = parts.length; depth > 0; depth--) {
      const dir = await directory(this.root, parts.slice(0, depth - 1), false).catch((error: unknown) => {
        if (error instanceof DOMException && error.name === 'NotFoundError') return null
        throw error
      })
      if (!dir) return
      try {
        await dir.removeEntry(parts[depth - 1])
      } catch (error) {
        if (error instanceof DOMException && error.name === 'NotFoundError') continue
        // A directory that still holds files stays.
        if (error instanceof DOMException && error.name === 'InvalidModificationError') return
        throw error
      }
    }
  }

  async scan(): Promise<Signature> {
    try {
      const signature: Signature = {}
      const files: [string, FileSystemFileHandle][] = []
      for await (const found of walk(this.root)) files.push(found)
      await each(files, async ([path, handle]) => {
        signature[path] = stamp(await handle.getFile())
      })
      return signature
    } catch (error) {
      throw classify(error)
    }
  }

  async copyBlobsFrom(source: ProjectStore): Promise<void> {
    if (!(source instanceof FolderBackend)) throw new StoreError('io', 'the source of a blob copy is not a folder')
    try {
      let from: FileSystemDirectoryHandle
      try {
        from = await source.root.getDirectoryHandle('blobs')
      } catch (error) {
        if (error instanceof DOMException && error.name === 'NotFoundError') return
        throw error
      }
      const to = await this.root.getDirectoryHandle('blobs', { create: true })
      const have = new Set<string>()
      for await (const entry of children(to)) have.add(entry.name)
      for await (const entry of children(from)) {
        if (entry.kind !== 'file' || have.has(entry.name)) continue
        const file = await (entry as FileSystemFileHandle).getFile()
        const writable = await (await to.getFileHandle(entry.name, { create: true })).createWritable()
        try {
          // A blob is named by its content: one left half written would pass for the real one.
          // Chunk by chunk: a blob is never held whole.
          const reader = file.stream().getReader()
          for (;;) {
            const { done, value } = await reader.read()
            if (done) break
            await writable.write(value as Uint8Array<ArrayBuffer>)
          }
          await writable.close()
        } catch (error) {
          await writable.abort().catch(() => undefined)
          await to.removeEntry(entry.name).catch(() => undefined)
          throw error
        }
      }
    } catch (error) {
      throw classify(error)
    }
  }

  async cleanUp(): Promise<void> {
    const sweep = async (dir: FileSystemDirectoryHandle, depth: number) => {
      const found: string[] = []
      for await (const entry of children(dir)) {
        if (entry.kind === 'file' && entry.name.endsWith(SWAP_SUFFIX)) found.push(entry.name)
        else if (entry.kind === 'directory' && depth < 6 && !SKIPPED_DIRS.includes(entry.name)) await sweep(entry as FileSystemDirectoryHandle, depth + 1)
      }
      for (const name of found) await dir.removeEntry(name).catch(() => undefined)
    }
    try {
      await sweep(this.root, 0)
      // What an interrupted creation left in `.faulk/tmp/`.
      const tmp = await directory(this.root, ['.faulk', 'tmp'], false)
      const left: string[] = []
      for await (const entry of children(tmp)) left.push(entry.name)
      for (const name of left) await tmp.removeEntry(name).catch(() => undefined)
    } catch {
      // Best effort: a swap file left behind is harmless.
    }
  }

  close() {}
}
