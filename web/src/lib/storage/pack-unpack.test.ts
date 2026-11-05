// The `.faulkpack` import in the store's Worker (#682): the pack read in the ranges the core asks for, each blob
// written into the folder as it is inflated and checked, the project's own files answered, and every way the
// import ends (whole, the project's files alone, damaged, cancelled, refused by the folder), with the real project
// module and a folder in memory, then through the Worker's own message handler (`worker.ts`).
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { coreModule, PACK_CHUNK_BYTES, ProjectCore, type FolderEntry, type PackUnpack } from '../project/core'
import { readFolder, SONG_FOLDER } from '../project/reference-folder'
import { FolderBackend } from './backend'
import { MemoryDir } from './memory-dir'
import type { Reply, Request } from './rpc'
import { StoreError } from './types'

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
const BLOB_BYTES = PACK_CHUNK_BYTES * 2 + (PACK_CHUNK_BYTES >> 1)
const media = () => new Uint8Array(BLOB_BYTES).map((_, i) => (i * 31 + (i >> 8)) & 0xff)
const HASH = createHash('sha256').update(media()).digest('hex')
const BLOB = `blobs/${HASH}`
const module_ = () => Promise.resolve(module)

/** The reference song, its audio blob renamed after a real one of three pieces, as the page stages it for an export. */
const song = (): FolderEntry[] => {
  const encoder = new TextEncoder()
  const decoder = new TextDecoder('utf-8', { fatal: true })
  return readFolder(SONG_FOLDER).map((entry) => {
    if (!('bytes' in entry) || entry.path.startsWith('blobs/')) return entry
    return { path: entry.path, bytes: encoder.encode(decoder.decode(entry.bytes).replaceAll('ab'.repeat(32), HASH)) }
  })
}

/** The pack of the song with its blob, as the page's core makes it. */
async function songPack(): Promise<Uint8Array> {
  const core = ProjectCore.create(module, 'empty', 1, seed)
  const parts: Uint8Array[] = []
  const done = await core.packExport([...song(), { path: BLOB, size: BLOB_BYTES }], {
    read: async (_path, start, end) => media().slice(start, end),
    sink: { write: async (chunk) => void parts.push(chunk) },
  })
  core.free()
  expect(done).toMatchObject({ status: 'ok', missingBlobs: 0 })
  const out = new Uint8Array(parts.reduce((n, part) => n + part.length, 0))
  let at = 0
  for (const part of parts) {
    out.set(part, at)
    at += part.length
  }
  return out
}

const fileOf = (bytes: Uint8Array) => new File([bytes as Uint8Array<ArrayBuffer>], 'song.faulkpack')

describe('FolderBackend.unpack', () => {
  it('writes every blob into the folder and answers the project files, which it does not write', async () => {
    const pack = await songPack()
    const dir = new MemoryDir('import')
    const progress: number[] = []
    const result = await new FolderBackend(dir.handle()).unpack(fileOf(pack), { onProgress: (f) => progress.push(f) }, module_)
    expect(result.status).toBe('ok')
    const files = (result as Extract<PackUnpack, { status: 'ok' }>).files
    // The project's own files: the song's, byte for byte, and no blob.
    const expected = song().filter((e) => !e.path.startsWith('blobs/') && !e.path.startsWith('.faulk/'))
    expect(files.map((f) => f.path).sort()).toEqual(expected.map((e) => e.path).sort())
    for (const file of files) expect(Array.from(file.bytes)).toEqual(Array.from((expected.find((e) => e.path === file.path) as { bytes: Uint8Array }).bytes))
    // The blobs, in the folder: the big one, and the song's lane blob.
    expect(Array.from(dir.bytes(BLOB)!)).toEqual(Array.from(media()))
    const lane = readFolder(SONG_FOLDER).find((e) => e.path.startsWith('blobs/'))!
    expect(Array.from(dir.bytes(lane.path)!)).toEqual(Array.from((lane as { bytes: Uint8Array }).bytes))
    expect(dir.paths().filter((p) => !p.startsWith('blobs/'))).toEqual([])
    // Progress grows to the whole.
    expect(progress.at(-1)).toBe(1)
    expect(progress.every((f, i) => i === 0 || f >= progress[i - 1]!)).toBe(true)
  })

  it('reads the project files alone, before any blob, when asked to', async () => {
    const pack = await songPack()
    const dir = new MemoryDir('import')
    const result = await new FolderBackend(dir.handle()).unpack(fileOf(pack), { textOnly: true }, module_)
    expect(result.status).toBe('ok')
    expect((result as Extract<PackUnpack, { status: 'ok' }>).files.some((f) => f.path === 'project.json')).toBe(true)
    expect(dir.paths()).toEqual([])
  })

  it('reads a pack downloaded into the folder by its path', async () => {
    const pack = await songPack()
    const dir = new MemoryDir('import').put('.faulk/tmp/download.faulkpack', pack)
    const result = await new FolderBackend(dir.handle()).unpack('.faulk/tmp/download.faulkpack', {}, module_)
    expect(result.status).toBe('ok')
    expect(dir.bytes(BLOB)?.length).toBe(BLOB_BYTES)
    await expect(new FolderBackend(dir.handle()).unpack('.faulk/tmp/gone.faulkpack', {}, module_)).rejects.toMatchObject({ code: 'missing' })
  })

  it('fails with the reason on a damaged pack, and leaves no blob cut short under its name', async () => {
    const pack = await songPack()
    // A byte of the big blob's data changed: its hash says so at its end.
    const damaged = pack.slice()
    damaged[Math.floor(pack.length / 2)] ^= 0x55
    const dir = new MemoryDir('import')
    const result = await new FolderBackend(dir.handle()).unpack(fileOf(damaged), {}, module_)
    expect(result).toMatchObject({ status: 'failed', message: expect.stringContaining('bad .faulkpack') })
    expect(dir.bytes(BLOB)).toBeNull()
    // Not a pack at all; a pack cut short.
    expect(await new FolderBackend(new MemoryDir('x').handle()).unpack(fileOf(new TextEncoder().encode('not a zip')), {}, module_)).toMatchObject({
      status: 'failed',
    })
    expect(await new FolderBackend(new MemoryDir('x').handle()).unpack(fileOf(pack.slice(0, pack.length - 100)), {}, module_)).toMatchObject({
      status: 'failed',
    })
  })

  it('is cancelled before it starts and between two pieces, and leaves no blob cut short', async () => {
    const pack = await songPack()
    const before = new AbortController()
    before.abort()
    expect(await new FolderBackend(new MemoryDir('x').handle()).unpack(fileOf(pack), { signal: before.signal }, module_)).toEqual({ status: 'cancelled' })
    const dir = new MemoryDir('import')
    const midway = new AbortController()
    const result = await new FolderBackend(dir.handle()).unpack(
      fileOf(pack),
      {
        signal: midway.signal,
        onProgress: (f) => {
          if (f > 0.5) midway.abort()
        },
      },
      module_,
    )
    expect(result).toEqual({ status: 'cancelled' })
    expect(dir.bytes(BLOB)).toBeNull()
  })

  it('throws what the folder refused (a full browser), with no blob cut short', async () => {
    const pack = await songPack()
    const dir = new MemoryDir('import')
    let writes = 0
    dir.hooks.failWrite = (name) => name === HASH && ++writes > 1
    await expect(new FolderBackend(dir.handle()).unpack(fileOf(pack), {}, module_)).rejects.toEqual(expect.any(StoreError))
    await expect(new FolderBackend(dir.handle()).unpack(fileOf(pack), {}, module_)).rejects.toMatchObject({ code: 'full' })
    expect(dir.bytes(BLOB)).toBeNull()
  })

  it('writes a file in pieces, whole at its close, nothing at its abort', async () => {
    const dir = new MemoryDir('x')
    const backend = new FolderBackend(dir.handle())
    const id = await backend.openWriter('.faulk/tmp/download.faulkpack')
    await backend.writePiece(id, new Uint8Array([1, 2]))
    await backend.writePiece(id, new Uint8Array([3]))
    expect(dir.bytes('.faulk/tmp/download.faulkpack')).toEqual(new Uint8Array())
    await backend.closeWriter(id)
    expect(dir.bytes('.faulk/tmp/download.faulkpack')).toEqual(new Uint8Array([1, 2, 3]))
    await expect(backend.writePiece(id, new Uint8Array([4]))).rejects.toMatchObject({ code: 'io' })
    const other = await backend.openWriter('.faulk/tmp/other')
    await backend.writePiece(other, new Uint8Array([9]))
    await backend.abortWriter(other)
    expect(dir.bytes('.faulk/tmp/other')).toBeNull()
    // A writer the page left open goes with the folder.
    const left = await backend.openWriter('.faulk/tmp/left')
    await backend.writePiece(left, new Uint8Array([9]))
    backend.close()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(dir.bytes('.faulk/tmp/left')).toBeNull()
  })
})

describe('the store Worker\'s "unpack" messages', () => {
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
  const final = (id: number) => posted.find((p) => p.reply.id === id && 'ok' in p.reply)?.reply

  it('imports a file, telling how far it is, and hands the project files over', async () => {
    const dir = new MemoryDir('import')
    await send({ op: 'open', key: 3, handle: dir.handle() as unknown as FileSystemDirectoryHandle })
    const id = next
    await send({ op: 'unpack', key: 3, source: fileOf(await songPack()) })
    const reply = final(id) as { ok: true; value: PackUnpack }
    expect(reply.value.status).toBe('ok')
    expect(posted.some((p) => p.reply.id === id && 'progress' in p.reply)).toBe(true)
    expect(posted.find((p) => p.reply === reply)!.transfer.length).toBeGreaterThan(0)
    expect(dir.bytes(BLOB)?.length).toBe(BLOB_BYTES)
  })

  it('ends cancelled when the page cancels it', async () => {
    const dir = new MemoryDir('import')
    await send({ op: 'open', key: 3, handle: dir.handle() as unknown as FileSystemDirectoryHandle })
    const id = next
    const running = send({ op: 'unpack', key: 3, source: fileOf(await songPack()) })
    // The cancel arrives while the Worker reads (between two of its awaits).
    await send({ op: 'cancel', target: id })
    await running
    expect((final(id) as { ok: true; value: PackUnpack }).value).toEqual({ status: 'cancelled' })
    expect(dir.bytes(BLOB)).toBeNull()
  })

  it('answers a folder it was never given with an error', async () => {
    const id = next
    await send({ op: 'unpack', key: 404, source: fileOf(new Uint8Array(1)) })
    expect(final(id)).toMatchObject({ ok: false })
  })
})
