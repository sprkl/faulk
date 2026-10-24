// The page's side of the project store's Worker (SV-2): `ProjectStore` over `postMessage`.
import type { FolderEntry, SaveEntry } from '../project/core'
import { StoreError, type FolderHandle, type ProjectStore, type Signature, type StoreErrorCode } from './types'
import { FolderBackend } from './backend'

/** The calls of `ProjectStore` the Worker takes by name. */
export const STORE_METHODS = ['isEmpty', 'hasProject', 'readFolder', 'read', 'apply', 'scan', 'copyBlobsFrom', 'cleanUp'] as const
type Method = (typeof STORE_METHODS)[number]

export type Request =
  | { id: number; op: 'open'; key: number; handle: FileSystemDirectoryHandle }
  | { id: number; op: 'close'; key: number }
  | { id: number; op: 'call'; key: number; method: Method; args: unknown[] }

type RequestBody = Request extends infer R ? (R extends unknown ? Omit<R, 'id'> : never) : never

export type Reply = { id: number; ok: true; value: unknown } | { id: number; ok: false; code: StoreErrorCode; message: string }

/** The one Worker, and the requests waiting for its answer. */
class StoreWorker {
  private next = 1
  private nextKey = 1
  private readonly waiting = new Map<number, { resolve(value: unknown): void; reject(error: StoreError): void }>()

  constructor(private readonly worker: Worker) {
    worker.onmessage = (event: MessageEvent<Reply>) => {
      const reply = event.data
      const waiter = this.waiting.get(reply.id)
      if (!waiter) return
      this.waiting.delete(reply.id)
      if (reply.ok) waiter.resolve(reply.value)
      else waiter.reject(new StoreError(reply.code, reply.message))
    }
    // A Worker that dies leaves its requests unanswered: fail them, the page then shows a save error.
    worker.onerror = (event) => {
      const error = new StoreError('io', `the project store stopped: ${event.message}`)
      for (const waiter of this.waiting.values()) waiter.reject(error)
      this.waiting.clear()
    }
  }

  request<T>(message: RequestBody, transfer: Transferable[] = []): Promise<T> {
    const id = this.next++
    return new Promise<T>((resolve, reject) => {
      this.waiting.set(id, { resolve: resolve as (value: unknown) => void, reject })
      this.worker.postMessage({ ...message, id }, transfer)
    })
  }

  async open(handle: FolderHandle): Promise<WorkerStore> {
    const key = this.nextKey++
    const name = await this.request<string>({ op: 'open', key, handle })
    return new WorkerStore(this, key, name)
  }
}

class WorkerStore implements ProjectStore {
  constructor(
    private readonly worker: StoreWorker,
    readonly key: number,
    readonly name: string,
  ) {}

  private call<T>(method: Method, args: unknown[] = [], transfer: Transferable[] = []): Promise<T> {
    return this.worker.request<T>({ op: 'call', key: this.key, method, args }, transfer)
  }

  isEmpty() {
    return this.call<boolean>('isEmpty')
  }
  hasProject() {
    return this.call<boolean>('hasProject')
  }
  readFolder() {
    return this.call<{ entries: FolderEntry[]; signature: Signature }>('readFolder')
  }
  read(path: string, start?: number, end?: number) {
    return this.call<Uint8Array>('read', [path, start, end])
  }
  apply(entries: SaveEntry[]) {
    const transfer = entries.flatMap((e) => (e.type === 'write' ? [e.bytes.buffer as ArrayBuffer] : []))
    return this.call<Signature>('apply', [entries], transfer)
  }
  scan() {
    return this.call<Signature>('scan')
  }
  copyBlobsFrom(source: ProjectStore) {
    if (!(source instanceof WorkerStore)) return Promise.reject(new StoreError('io', 'the source of a blob copy is not a folder'))
    return this.call<void>('copyBlobsFrom', [source.key])
  }
  cleanUp() {
    return this.call<void>('cleanUp')
  }
  close() {
    void this.worker.request({ op: 'close', key: this.key }).catch(() => undefined)
  }
}

let shared: StoreWorker | null = null

/**
 * Opens a folder in the store's Worker, or in the page itself where Workers do not exist (a
 * test, an old browser). One Worker serves every folder the page opens. A browser check sets
 * `__faulkStoreInPage` to hold a write open from the page, which a Worker's scope hides from it.
 */
export async function openStore(handle: FolderHandle): Promise<ProjectStore> {
  if (typeof Worker === 'undefined' || (globalThis as { __faulkStoreInPage?: boolean }).__faulkStoreInPage) return new FolderBackend(handle)
  shared ??= new StoreWorker(new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' }))
  return shared.open(handle)
}
