import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { StoreWorker, STORE_TIMEOUT_MS, WorkerStore, type Reply, type WorkerLike } from './rpc'
import { StoreError, type FolderHandle } from './types'

/** A Worker that answers `open` and nothing else until told to, and records what it was sent. */
class FakeWorker implements WorkerLike {
  onmessage: ((event: MessageEvent<Reply>) => void) | null = null
  onerror: ((event: ErrorEvent) => void) | null = null
  sent: { id: number; op: string; method?: string; key?: number }[] = []
  terminated = false
  answers = true
  postMessage(message: unknown) {
    const request = message as { id: number; op: string; method?: string; key?: number }
    this.sent.push(request)
    if (!this.answers && request.op === 'call') return
    const value = request.op === 'open' ? 'folder' : request.method === 'isEmpty' ? true : null
    queueMicrotask(() => this.onmessage?.({ data: { id: request.id, ok: true, value } } as MessageEvent<Reply>))
  }
  terminate() {
    this.terminated = true
  }
}

const handle = { name: 'folder' } as unknown as FolderHandle

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe('the store worker', () => {
  it('answers calls by key, and a refusal arrives as a StoreError with its code', async () => {
    const fake = new FakeWorker()
    const worker = new StoreWorker(fake)
    const { key, name } = await worker.openKey(handle)
    expect(name).toBe('folder')
    expect(await worker.request<boolean>({ op: 'call', key, method: 'isEmpty', args: [] })).toBe(true)
    const pending = worker.request({ op: 'call', key, method: 'scan', args: [] })
    fake.onmessage?.({ data: { id: fake.sent.at(-1)!.id, ok: false, code: 'permission', message: 'no' } } as MessageEvent<Reply>)
    await expect(pending).rejects.toMatchObject({ code: 'permission' })
  })

  it('drops a Worker that does not answer: the call fails, the Worker ends, later calls fail at once', async () => {
    const fake = new FakeWorker()
    const worker = new StoreWorker(fake)
    const { key } = await worker.openKey(handle)
    fake.answers = false
    const lost = worker.request({ op: 'call', key, method: 'apply', args: [[]] })
    const caught = expect(lost).rejects.toBeInstanceOf(StoreError)
    await vi.advanceTimersByTimeAsync(STORE_TIMEOUT_MS)
    await caught
    expect(worker.dead).toBe(true)
    expect(fake.terminated).toBe(true)
    await expect(worker.request({ op: 'call', key, method: 'scan', args: [] })).rejects.toBeInstanceOf(StoreError)
  })

  it('fails the waiting calls when the Worker reports an error', async () => {
    const fake = new FakeWorker()
    const worker = new StoreWorker(fake)
    const { key } = await worker.openKey(handle)
    fake.answers = false
    const lost = worker.request({ op: 'call', key, method: 'scan', args: [] })
    const caught = expect(lost).rejects.toMatchObject({ code: 'io' })
    fake.onerror?.({ message: 'boom' } as ErrorEvent)
    await caught
    expect(worker.dead).toBe(true)
  })

  it('a folder whose Worker was dropped is opened again in the next one, and the call goes through', async () => {
    const first = new FakeWorker()
    const second = new FakeWorker()
    const dead = new StoreWorker(first)
    const next = new StoreWorker(second)
    const { key, name } = await dead.openKey(handle)
    const store = new WorkerStore(dead, key, name, handle, () => next)
    first.onerror?.({ message: 'gone' } as ErrorEvent)
    expect(await store.isEmpty()).toBe(true)
    expect(second.sent.map((m) => m.op)).toEqual(['open', 'call'])
  })
})
