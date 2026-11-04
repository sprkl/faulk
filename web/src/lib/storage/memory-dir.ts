// Test helper: a directory in memory with the shape of a `FileSystemDirectoryHandle` (the part the
// project store uses), and the behaviour the store relies on: a writable stream changes nothing
// until it is closed. Failures can be injected, to attack the store's write path.
import type { FolderHandle } from './types'

type Node = MemoryDir | MemoryFile

let clock = 1_700_000_000_000

export class MemoryFile {
  readonly kind = 'file'
  bytes = new Uint8Array()
  modified = clock++
  constructor(readonly name: string) {}

  /** Moves the file into `dir` under `name` (the part of `FileSystemFileHandle.move` the store uses). */
  async move(dir: MemoryDir, name: string) {
    const from = (this as { parent?: MemoryDir }).parent
    if (from?.root.hooks.noMove) throw new DOMException('move is not supported here', 'NotSupportedError')
    if (from?.root.hooks.moveRejects) throw new DOMException('move refused', from.root.hooks.moveRejects)
    from?.entries.delete(this.name)
    ;(this as { name: string }).name = name
    ;(this as { parent?: MemoryDir }).parent = dir
    dir.entries.set(name, this)
  }

  async getFile(): Promise<File> {
    return new File([this.bytes as Uint8Array<ArrayBuffer>], this.name, { lastModified: this.modified })
  }

  async createWritable(): Promise<FileSystemWritableFileStream> {
    const hooks = (this as { parent?: MemoryDir }).parent?.root.hooks
    const name = this.name
    const commit = (bytes: Uint8Array<ArrayBuffer>) => {
      this.bytes = bytes
      this.modified = clock++
    }
    // The swap file: written at a cursor (or at a `position`, as `FileSystemWritableFileStream` takes it).
    let swap = new Uint8Array(0)
    let length = 0
    let cursor = 0
    let finished = false
    const put = (data: Uint8Array, at: number) => {
      const end = at + data.length
      if (end > swap.length) {
        const grown = new Uint8Array(Math.max(end, swap.length * 2))
        grown.set(swap.subarray(0, length))
        swap = grown
      }
      swap.set(data, at)
      length = Math.max(length, end)
      cursor = end
    }
    const stream = {
      async write(data: Uint8Array | { type: 'write'; position?: number; data: Uint8Array }) {
        if (hooks?.failWrite?.(name)) throw new DOMException('disk full', 'QuotaExceededError')
        if (data instanceof Uint8Array) put(new Uint8Array(data), cursor)
        else put(new Uint8Array(data.data), data.position ?? cursor)
      },
      async close() {
        if (finished) throw new DOMException('closed', 'InvalidStateError')
        if (hooks?.failClose?.(name)) {
          finished = true
          throw new DOMException('the browser refused the file', 'AbortError')
        }
        finished = true
        commit(swap.slice(0, length))
      },
      async abort() {
        finished = true
      },
      // `pipeTo` target
      getWriter() {
        return {
          write: (data: Uint8Array) => stream.write(data),
          close: () => stream.close(),
          abort: () => stream.abort(),
          releaseLock() {},
          closed: Promise.resolve(),
        }
      },
      locked: false,
    }
    return stream as unknown as FileSystemWritableFileStream
  }
}

export interface Hooks {
  failWrite?: (file: string) => boolean
  failClose?: (file: string) => boolean
  failEntries?: () => DOMException | null
  /** `move` is not available: the store falls back to making the file in place. */
  noMove?: boolean
  /** `move` exists but rejects with a DOMException of this name. */
  moveRejects?: string
}

export class MemoryDir {
  readonly kind = 'directory'
  readonly entries = new Map<string, Node>()
  parent: MemoryDir | null = null
  hooks: Hooks = {}
  constructor(readonly name: string) {}

  get root(): MemoryDir {
    return this.parent ? this.parent.root : this
  }

  async getFileHandle(name: string, options?: { create?: boolean }): Promise<MemoryFile> {
    const found = this.entries.get(name)
    if (found instanceof MemoryFile) return found
    if (found) throw new DOMException('not a file', 'TypeMismatchError')
    if (!options?.create) throw new DOMException(`${name} not found`, 'NotFoundError')
    const file = new MemoryFile(name)
    ;(file as { parent?: MemoryDir }).parent = this
    this.entries.set(name, file)
    return file
  }

  async getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<MemoryDir> {
    const found = this.entries.get(name)
    if (found instanceof MemoryDir) return found
    if (found) throw new DOMException('not a directory', 'TypeMismatchError')
    if (!options?.create) throw new DOMException(`${name} not found`, 'NotFoundError')
    const dir = new MemoryDir(name)
    dir.parent = this
    this.entries.set(name, dir)
    return dir
  }

  async removeEntry(name: string, options?: { recursive?: boolean }) {
    const found = this.entries.get(name)
    if (!found) throw new DOMException(`${name} not found`, 'NotFoundError')
    if (found instanceof MemoryDir && found.entries.size > 0 && !options?.recursive) throw new DOMException('not empty', 'InvalidModificationError')
    this.entries.delete(name)
  }

  async *values(): AsyncGenerator<Node> {
    const failure = this.root.hooks.failEntries?.()
    if (failure) throw failure
    yield* [...this.entries.values()]
  }

  async isSameEntry(other: unknown) {
    return other === this
  }

  // ------------------------------------------------------------------ helpers for tests

  /** Puts a file at `path`, making the directories. */
  put(path: string, content: string | Uint8Array): this {
    const parts = path.split('/')
    const dir = parts.slice(0, -1).reduce<MemoryDir>((parent, part) => {
      let next = parent.entries.get(part)
      if (!(next instanceof MemoryDir)) {
        next = new MemoryDir(part)
        next.parent = parent
        parent.entries.set(part, next)
      }
      return next
    }, this)
    const file = new MemoryFile(parts[parts.length - 1]!)
    ;(file as { parent?: MemoryDir }).parent = dir
    file.bytes = typeof content === 'string' ? new TextEncoder().encode(content) : (content as Uint8Array<ArrayBuffer>)
    dir.entries.set(file.name, file)
    return this
  }

  /** The text of the file at `path`, or `null`. */
  text(path: string): string | null {
    const node = path.split('/').reduce<Node | undefined>((at, part) => (at instanceof MemoryDir ? at.entries.get(part) : undefined), this)
    return node instanceof MemoryFile ? new TextDecoder().decode(node.bytes) : null
  }

  /** A copy of the bytes of the file at `path`, or `null`. */
  bytes(path: string): Uint8Array | null {
    const node = path.split('/').reduce<Node | undefined>((at, part) => (at instanceof MemoryDir ? at.entries.get(part) : undefined), this)
    return node instanceof MemoryFile ? node.bytes.slice() : null
  }

  /** Removes the file at `path`, if there is one. */
  remove(path: string): void {
    const parts = path.split('/')
    const dir = parts.slice(0, -1).reduce<Node | undefined>((at, part) => (at instanceof MemoryDir ? at.entries.get(part) : undefined), this)
    if (dir instanceof MemoryDir) dir.entries.delete(parts[parts.length - 1]!)
  }

  /** Every file path below this directory. */
  paths(base = ''): string[] {
    return [...this.entries].flatMap(([name, node]) =>
      node instanceof MemoryDir ? node.paths(base ? `${base}/${name}` : name) : [base ? `${base}/${name}` : name],
    )
  }

  /** As the handle type the store takes. */
  handle(): FolderHandle {
    return this as unknown as FolderHandle
  }
}
