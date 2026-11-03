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
    if (!this.answers && request.op !== 'open') return
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

describe('an audio import through the store worker (AU-1)', () => {
  /** A Worker that answers `open` and leaves an import to the test. */
  const importing = async () => {
    const fake = new FakeWorker()
    const worker = new StoreWorker(fake)
    const store = new WorkerStore(worker, (await worker.openKey(handle)).key, 'folder', handle, () => worker)
    fake.answers = false
    return { fake, worker, store }
  }
  const reply = (fake: FakeWorker, data: Reply) => fake.onmessage?.({ data } as MessageEvent<Reply>)

  it('hands the progress over, and a long import is kept alive by it past the deadline', async () => {
    const { fake, worker, store } = await importing()
    const seen: number[] = []
    const file = new File([new Uint8Array(4)], 'kick.wav')
    const pending = store.importAudio(file, (f) => seen.push(f))
    await vi.advanceTimersByTimeAsync(0)
    const request = fake.sent.at(-1)!
    expect(request.op).toBe('import')
    // Well past the deadline in all, but never that long without a word.
    for (let i = 1; i <= 4; i++) {
      await vi.advanceTimersByTimeAsync(STORE_TIMEOUT_MS - 1000)
      reply(fake, { id: request.id, progress: i / 4 })
    }
    expect(worker.dead).toBe(false)
    expect(seen).toEqual([0.25, 0.5, 0.75, 1])
    const outcome = { media: { hash: 'a'.repeat(64) }, direct: true, blob: { path: 'blobs/x', stamp: '1:1', created: true } }
    reply(fake, { id: request.id, ok: true, value: outcome })
    await expect(pending).resolves.toEqual(outcome)
    // Silence for the whole deadline still drops the Worker.
    const lost = store.importAudio(file)
    const caught = expect(lost).rejects.toBeInstanceOf(StoreError)
    await vi.advanceTimersByTimeAsync(STORE_TIMEOUT_MS)
    await caught
    expect(worker.dead).toBe(true)
  })

  it('a file the import refuses arrives as an ImportError with its code; a folder that refused a write as a StoreError', async () => {
    const { fake, store } = await importing()
    const refused = store.importAudio(new File([], 'song.opus'))
    await vi.advanceTimersByTimeAsync(0)
    reply(fake, { id: fake.sent.at(-1)!.id, ok: false, code: 'io', message: 'Opus', importCode: 'unsupported_codec' })
    await expect(refused).rejects.toMatchObject({ name: 'ImportError', code: 'unsupported_codec' })
    const full = store.importAudio(new File([], 'big.wav'))
    await vi.advanceTimersByTimeAsync(0)
    reply(fake, { id: fake.sent.at(-1)!.id, ok: false, code: 'full', message: 'quota' })
    await expect(full).rejects.toMatchObject({ name: 'StoreError', code: 'full' })
  })

  it('a cancel is sent for the import it names, and a late progress after the answer is ignored', async () => {
    const { fake, store } = await importing()
    const abort = new AbortController()
    const pending = store.importAudio(new File([], 'a.wav'), undefined, abort.signal)
    await vi.advanceTimersByTimeAsync(0)
    const request = fake.sent.at(-1)!
    abort.abort()
    expect(fake.sent.at(-1)).toMatchObject({ op: 'cancel', target: request.id })
    reply(fake, { id: request.id, ok: false, code: 'io', message: 'cancelled', importCode: 'cancelled' })
    await expect(pending).rejects.toMatchObject({ code: 'cancelled' })
    // Nothing waits for it any more: a stray message changes nothing, and a later abort sends nothing.
    reply(fake, { id: request.id, progress: 0.5 })
    const sent = fake.sent.length
    abort.abort()
    expect(fake.sent.length).toBe(sent)
  })
})
