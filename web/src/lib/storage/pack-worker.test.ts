// The `.faulkpack` export in the store's Worker (#685): the zip pieces the Worker makes, the page's word between two
// pieces, and every way the export ends (whole, cancelled, refused by the page, failed), through the Worker's own
// message handler (`worker.ts`) with the real project module and a folder in memory.
import { readFileSync } from 'node:fs'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { coreModule, PACK_CHUNK_BYTES, ProjectCore, type FolderEntry, type PackExport } from '../project/core'
import { readFolder, SONG_FOLDER } from '../project/reference-folder'
import { FolderBackend } from './backend'
import { MemoryDir } from './memory-dir'
import { packInWorker } from './pack-worker'
import type { Reply, Request } from './rpc'

let module: WebAssembly.Module
beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('../project/wasm/project.wasm', import.meta.url)))
})

// The Worker compiles the module by `fetch`, which a test has no server for: it gets the bytes of the file.
vi.mock('../project/core', async (original) => ({
  ...(await original<typeof import('../project/core')>()),
  coreModule: () => WebAssembly.compile(readFileSync(new URL('../project/wasm/project.wasm', import.meta.url))),
}))

const seed = { timeMs: 1_760_000_000_000, random: [1, 2, 3] as [number, number, number] }
const BLOB = `blobs/${'ab'.repeat(32)}`
const BLOB_BYTES = PACK_CHUNK_BYTES * 2 + (PACK_CHUNK_BYTES >> 1)
const media = () => new Uint8Array(BLOB_BYTES).map((_, i) => (i * 31 + (i >> 8)) & 0xff)

/** The reference song and a blob of three pieces, as the page stages them for an export. */
const entries = (): FolderEntry[] => [...readFolder(SONG_FOLDER), { path: BLOB, size: BLOB_BYTES }]

const join = (parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, part) => n + part.length, 0))
  let at = 0
  for (const part of parts) {
    out.set(part, at)
    at += part.length
  }
  return out
}

describe('packInWorker', () => {
  const deps = (over: Partial<Parameters<typeof packInWorker>[2]> = {}) => ({
    read: async (_path: string, start: number, end: number) => media().slice(start, end),
    send: async () => undefined,
    progress: () => undefined,
    module: () => Promise.resolve(module),
    ...over,
  })

  it('makes the pack the page would make, one piece at a time, each written before the next is read', async () => {
    const core = ProjectCore.create(module, 'empty', 1, seed)
    const inPage: Uint8Array[] = []
    const expected = await core.packExport(entries(), {
      read: async (_path, start, end) => media().slice(start, end),
      sink: { write: async (chunk) => void inPage.push(chunk) },
    })
    core.free()
    const parts: Uint8Array[] = []
    let sending = false
    let reads = 0
    const result = await packInWorker(
      entries(),
      new AbortController().signal,
      deps({
        read: async (_path, start, end) => {
          // No piece is read while the page still writes the last one.
          expect(sending).toBe(false)
          reads++
          return media().slice(start, end)
        },
        send: async (chunk) => {
          sending = true
          await new Promise((resolve) => setTimeout(resolve, 1))
          parts.push(chunk)
          sending = false
        },
      }),
    )
    expect(result).toEqual(expected)
    expect(reads).toBe(3)
    expect(Array.from(join(parts))).toEqual(Array.from(join(inPage)))
  })

  it('is cancelled before it starts, between two pieces, and when the page stops it while it writes one', async () => {
    const early = new AbortController()
    early.abort()
    expect(await packInWorker(entries(), early.signal, deps())).toEqual({ status: 'cancelled' })

    const control = new AbortController()
    let sends = 0
    const cut = await packInWorker(
      entries(),
      control.signal,
      deps({
        send: async () => {
          if (++sends === 2) control.abort()
        },
      }),
    )
    expect(cut).toEqual({ status: 'cancelled' })
    // No piece is read after the abort: one send per read at most, the blob's second piece not read.
    expect(sends).toBeLessThanOrEqual(3)
  })

  it('fails with the reason when a blob is not the size it was listed with, and a failing send is the caller error', async () => {
    const short = await packInWorker(entries(), new AbortController().signal, deps({ read: async () => new Uint8Array(3) }))
    expect(short).toMatchObject({ status: 'failed', message: expect.stringContaining('changed while it was exported') })
    await expect(
      packInWorker(
        entries(),
        new AbortController().signal,
        deps({
          send: async () => {
            throw new Error('disk full')
          },
        }),
      ),
    ).rejects.toThrow('disk full')
    // The session was ended each time: the next export of the same module is whole.
    expect((await packInWorker(entries(), new AbortController().signal, deps())).status).toBe('ok')
  })
})

describe('the store Worker\'s "pack" messages', () => {
  const posted: { reply: Reply; transfer: ArrayBuffer[] }[] = []
  let onmessage: (event: { data: Request }) => Promise<void>

  beforeEach(async () => {
    posted.length = 0
    vi.resetModules()
    const scope = { postMessage: (reply: Reply, transfer: ArrayBuffer[] = []) => void posted.push({ reply, transfer }), onmessage: null as unknown }
    vi.stubGlobal('self', scope)
    await import('./worker')
    onmessage = scope.onmessage as typeof onmessage
  })
  afterEach(() => vi.unstubAllGlobals())

  let next = 1
  const send = (request: Record<string, unknown>) => onmessage({ data: { ...request, id: next++ } as Request })
  const open = async () => {
    const dir = new MemoryDir('song')
    await new FolderBackend(dir.handle()).apply([{ type: 'write', path: BLOB, bytes: media() }])
    await send({ op: 'open', key: 7, handle: dir.handle() as unknown as FileSystemDirectoryHandle })
  }
  const chunks = () => posted.filter((p) => 'chunk' in p.reply)
  const final = (id: number) => posted.find((p) => p.reply.id === id && 'ok' in p.reply)?.reply

  /** Waits until the Worker has posted `count` pieces (it then waits for our word). */
  const untilChunks = async (count: number) => {
    for (let i = 0; i < 400 && chunks().length < count; i++) await new Promise((resolve) => setTimeout(resolve, 5))
    expect(chunks().length).toBeGreaterThanOrEqual(count)
  }

  it('hands each piece over and waits for the page before the next; the pieces make the whole pack', async () => {
    await open()
    const id = next
    const running = send({ op: 'pack', key: 7, entries: entries() })
    let seen = 0
    for (;;) {
      await untilChunksOrDone(id, seen + 1)
      if (final(id)) break
      // Nothing more comes until the ack: the Worker holds on a single piece.
      await new Promise((resolve) => setTimeout(resolve, 20))
      expect(chunks().length).toBe(seen + 1)
      seen++
      await send({ op: 'packAck', target: id })
    }
    await running
    const reply = final(id) as { ok: true; value: PackExport }
    expect(reply.value).toMatchObject({ status: 'ok', missingBlobs: 0 })
    const zip = join(chunks().map((p) => (p.reply as { chunk: Uint8Array }).chunk))
    // Each piece was handed over, not copied.
    expect(chunks().every((p) => p.transfer.length === 1)).toBe(true)
    // The same bytes as the page's own core makes of the same folder.
    const core = ProjectCore.create(module, 'empty', 1, seed)
    const inPage: Uint8Array[] = []
    await core.packExport(entries(), {
      read: async (_path, start, end) => media().slice(start, end),
      sink: { write: async (chunk) => void inPage.push(chunk) },
    })
    core.free()
    expect(Array.from(zip)).toEqual(Array.from(join(inPage)))
  })

  it("answers only after the page's word on the last piece, the one that ends the zip (#737)", async () => {
    await open()
    const id = next
    const running = send({ op: 'pack', key: 7, entries: entries() })
    let seen = 0
    for (;;) {
      await untilChunksOrDone(id, seen + 1)
      // Never an answer while a piece waits for the page, the last one included.
      expect(final(id)).toBeUndefined()
      seen++
      const last = (chunks().at(-1)!.reply as { chunk: Uint8Array }).chunk
      // The end of central directory record: the piece that completes the zip.
      const ends = last.length >= 22 && new DataView(last.buffer, last.byteOffset + last.length - 22, 4).getUint32(0, true) === 0x06054b50
      if (ends) {
        await new Promise((resolve) => setTimeout(resolve, 50))
        expect(final(id)).toBeUndefined()
        expect(chunks()).toHaveLength(seen)
        await send({ op: 'packAck', target: id })
        break
      }
      await send({ op: 'packAck', target: id })
    }
    await running
    expect((final(id) as { ok: true; value: PackExport }).value).toMatchObject({ status: 'ok' })
    // Nothing came after the answer.
    expect(posted.at(-1)!.reply).toMatchObject({ id, ok: true })
  })

  async function untilChunksOrDone(id: number, count: number) {
    for (let i = 0; i < 2000 && chunks().length < count && !final(id); i++) await new Promise((resolve) => setTimeout(resolve, 5))
  }

  it('ends cancelled, with no more pieces, when the page could not write one', async () => {
    await open()
    const id = next
    const running = send({ op: 'pack', key: 7, entries: entries() })
    await untilChunks(1)
    await send({ op: 'packAck', target: id, failed: true })
    await running
    expect((final(id) as { ok: true; value: PackExport }).value).toEqual({ status: 'cancelled' })
    expect(chunks()).toHaveLength(1)
  })

  it('ends cancelled when the page cancels while the Worker waits for it, and forgets the export', async () => {
    await open()
    const id = next
    const running = send({ op: 'pack', key: 7, entries: entries() })
    await untilChunks(1)
    await send({ op: 'cancel', target: id })
    await running
    expect((final(id) as { ok: true; value: PackExport }).value).toEqual({ status: 'cancelled' })
    // A late word for it, or a cancel of nothing, is ignored.
    await send({ op: 'packAck', target: id })
    await send({ op: 'cancel', target: id })
    await send({ op: 'packAck', target: 9999, failed: true })
    expect(posted.filter((p) => p.reply.id === id && 'ok' in p.reply)).toHaveLength(1)
  })

  it('answers a folder it was never given with an error, and a broken folder with the reason', async () => {
    const id = next
    await send({ op: 'pack', key: 404, entries: entries() })
    expect(final(id)).toMatchObject({ ok: false })
    await open()
    const bad = next
    await send({ op: 'pack', key: 7, entries: [{ path: 'README.md', bytes: new Uint8Array(1) }] })
    expect((final(bad) as { ok: true; value: PackExport }).value).toMatchObject({ status: 'failed' })
    expect(chunks()).toHaveLength(0)
  })
})
