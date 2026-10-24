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

/** A call the Worker has not answered after this long is lost: the Worker is dropped and the call fails. */
export const STORE_TIMEOUT_MS = 120_000

/** What the page needs of a Worker (a stand-in in tests). */
export interface WorkerLike {
  onmessage: ((event: MessageEvent<Reply>) => void) | null
  onerror: ((event: ErrorEvent) => void) | null
  postMessage(message: unknown, transfer?: Transferable[]): void
  terminate(): void
}

/** The one Worker, and the requests waiting for its answer. */
export class StoreWorker {
  /** Set once the Worker was dropped (it stopped, or stopped answering): a new one is made for the next call. */
  dead = false
  private next = 1
  private nextKey = 1
  private readonly waiting = new Map<number, { resolve(value: unknown): void; reject(error: StoreError): void }>()

  constructor(private readonly worker: WorkerLike) {
    worker.onmessage = (event) => {
      const reply = event.data
      const waiter = this.waiting.get(reply.id)
      if (!waiter) return
      this.waiting.delete(reply.id)
      if (reply.ok) waiter.resolve(reply.value)
      else waiter.reject(new StoreError(reply.code, reply.message))
    }
    worker.onerror = (event) => this.drop(`the project store stopped: ${event.message}`)
  }

  /** Fails every call waiting and forgets this Worker. */
  private drop(message: string) {
    this.dead = true
    const error = new StoreError('io', message)
    for (const waiter of this.waiting.values()) waiter.reject(error)
    this.waiting.clear()
    this.worker.terminate()
  }

  request<T>(message: RequestBody, transfer: Transferable[] = []): Promise<T> {
    if (this.dead) return Promise.reject(new StoreError('io', 'the project store stopped'))
    const id = this.next++
    return new Promise<T>((resolve, reject) => {
      // A Worker that never answers would hold the page's saves, new and open behind it for good.
      const timer = setTimeout(() => this.drop('the project store did not answer'), STORE_TIMEOUT_MS)
      this.waiting.set(id, {
        resolve: (value) => {
          clearTimeout(timer)
          resolve(value as T)
        },
        reject: (error) => {
          clearTimeout(timer)
          reject(error)
        },
      })
      this.worker.postMessage({ ...message, id }, transfer)
    })
  }

  /** Has the Worker open `handle`; the key names it in later calls. */
  async openKey(handle: FolderHandle): Promise<{ key: number; name: string }> {
    const key = this.nextKey++
    const name = await this.request<string>({ op: 'open', key, handle })
    return { key, name }
  }
}

export class WorkerStore implements ProjectStore {
  constructor(
    private worker: StoreWorker,
    private key: number,
    readonly name: string,
    private readonly handle: FolderHandle,
    private readonly replacement: () => StoreWorker,
  ) {}

  /** The key of this folder in a live Worker: a Worker that was dropped is replaced and the folder opened again in the new one. */
  async ready(): Promise<number> {
    if (this.worker.dead) {
      this.worker = this.replacement()
      this.key = (await this.worker.openKey(this.handle)).key
    }
    return this.key
  }

  private async call<T>(method: Method, args: unknown[] = [], transfer: Transferable[] = []): Promise<T> {
    const key = await this.ready()
    return this.worker.request<T>({ op: 'call', key, method, args }, transfer)
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
  async copyBlobsFrom(source: ProjectStore): Promise<void> {
    if (!(source instanceof WorkerStore)) throw new StoreError('io', 'the source of a blob copy is not a folder')
    const sourceKey: number = await source.ready()
    return this.call<void>('copyBlobsFrom', [sourceKey])
  }
  cleanUp() {
    return this.call<void>('cleanUp')
  }
  close() {
    void this.worker.request({ op: 'close', key: this.key }).catch(() => undefined)
  }
}

let shared: StoreWorker | null = null

/** The page's Worker for the store: made on first use, and again after one was dropped. */
function currentWorker(): StoreWorker {
  if (shared?.dead) shared = null
  shared ??= new StoreWorker(new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' }) as unknown as WorkerLike)
  return shared
}

/**
 * Opens a folder in the store's Worker, or in the page itself where Workers do not exist (a
 * test, an old browser). One Worker serves every folder the page opens. A browser check sets
 * `__faulkStoreInPage` to hold a write open from the page, which a Worker's scope hides from it.
 */
export async function openStore(handle: FolderHandle): Promise<ProjectStore> {
  if (typeof Worker === 'undefined' || (globalThis as { __faulkStoreInPage?: boolean }).__faulkStoreInPage) return new FolderBackend(handle)
  const worker = currentWorker()
  const { key, name } = await worker.openKey(handle)
  return new WorkerStore(worker, key, name, handle, currentWorker)
}
