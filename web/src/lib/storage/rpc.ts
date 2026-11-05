// The page's side of the project store's Worker (SV-2): `ProjectStore` over `postMessage`.
import { ImportError, type ImportErrorCode } from '../audio/import/abi'
import type { CacheReport, ImportOutcome, MediaRef } from '../audio/import/pipeline'
import type { FolderEntry, SaveEntry } from '../project/core'
import { StoreError, type FolderHandle, type ProjectStore, type Signature, type StoreErrorCode } from './types'
import { FolderBackend } from './backend'

/** The calls of `ProjectStore` the Worker takes by name. */
export const STORE_METHODS = ['isEmpty', 'hasProject', 'readFolder', 'listBlobs', 'read', 'apply', 'scan', 'copyBlobsFrom', 'cleanUp'] as const
type Method = (typeof STORE_METHODS)[number]

export type Request =
  | { id: number; op: 'open'; key: number; handle: FileSystemDirectoryHandle }
  | { id: number; op: 'close'; key: number }
  | { id: number; op: 'call'; key: number; method: Method; args: unknown[] }
  /** An audio import (AU-1): progress replies until the answer. */
  | { id: number; op: 'import'; key: number; file: File }
  /** The PCM caches a folder lacks, made again (AU-1). */
  | { id: number; op: 'audioCache'; key: number; media: MediaRef[] }
  /** Cancels the import `target` (no answer of its own). */
  | { id: number; op: 'cancel'; target: number }

type RequestBody = Request extends infer R ? (R extends unknown ? Omit<R, 'id'> : never) : never

export type Reply =
  | { id: number; ok: true; value: unknown }
  /** `importCode`: the import refused the file (an `ImportError`), rather than the folder a write. */
  | { id: number; ok: false; code: StoreErrorCode; message: string; importCode?: ImportErrorCode }
  /** How far a long call (an import) is, from 0 to 1. */
  | { id: number; progress: number }

/** A call the Worker has not answered after this long is lost: the Worker is dropped and the call fails. */
export const STORE_TIMEOUT_MS = 600_000

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
  private readonly waiting = new Map<number, { resolve(value: unknown): void; reject(error: Error): void; progress(fraction: number): void }>()

  constructor(private readonly worker: WorkerLike) {
    worker.onmessage = (event) => {
      const reply = event.data
      const waiter = this.waiting.get(reply.id)
      if (!waiter) return
      if ('progress' in reply) {
        waiter.progress(reply.progress)
        return
      }
      this.waiting.delete(reply.id)
      if (reply.ok) waiter.resolve(reply.value)
      else if (reply.importCode) waiter.reject(new ImportError(reply.importCode, reply.message))
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

  /**
   * Sends a request and waits for its answer. A call that reports progress (an import) keeps the
   * Worker alive with each report: the deadline counts from the last sign of life, not from the
   * start, so an hour-long import is not taken for a Worker that stopped. `signal` sends a cancel.
   */
  request<T>(message: RequestBody, transfer: Transferable[] = [], options: { onProgress?: (fraction: number) => void; signal?: AbortSignal } = {}): Promise<T> {
    if (this.dead) return Promise.reject(new StoreError('io', 'the project store stopped'))
    // Cancelled before it was sent (it waited behind other work): never sent, so nothing to undo.
    if (options.signal?.aborted) return Promise.reject(new ImportError('cancelled', 'cancelled before it started'))
    const id = this.next++
    return new Promise<T>((resolve, reject) => {
      // A Worker that never answers would hold the page's saves, new and open behind it for good.
      let timer = setTimeout(() => this.drop('the project store did not answer'), STORE_TIMEOUT_MS)
      const cancel = () => this.worker.postMessage({ op: 'cancel', target: id, id: this.next++ })
      options.signal?.addEventListener('abort', cancel, { once: true })
      const done = () => {
        clearTimeout(timer)
        options.signal?.removeEventListener('abort', cancel)
      }
      this.waiting.set(id, {
        resolve: (value) => {
          done()
          resolve(value as T)
        },
        reject: (error) => {
          done()
          reject(error)
        },
        progress: (fraction) => {
          clearTimeout(timer)
          timer = setTimeout(() => this.drop('the project store did not answer'), STORE_TIMEOUT_MS)
          options.onProgress?.(fraction)
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
  listBlobs() {
    return this.call<FolderEntry[]>('listBlobs')
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
  async importAudio(file: File, onProgress?: (fraction: number) => void, signal?: AbortSignal): Promise<ImportOutcome> {
    const key = await this.ready()
    return this.worker.request<ImportOutcome>({ op: 'import', key, file }, [], { onProgress, signal })
  }
  async ensureAudioCache(media: MediaRef[], onProgress?: (fraction: number) => void, signal?: AbortSignal): Promise<CacheReport> {
    const key = await this.ready()
    return this.worker.request<CacheReport>({ op: 'audioCache', key, media }, [], { onProgress: onProgress ?? (() => undefined), signal })
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
