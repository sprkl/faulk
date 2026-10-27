import { createServer, type Server } from 'node:http'
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs'
import { join, relative, sep } from 'node:path'
import { expect, test, type Page } from '@playwright/test'

// QA-7 (#130): the project folder in the browser's file system (OPFS) and the prototype reader
// compiled to wasm32. A measurement, not a test: run by hand with `pnpm bench:project` once the
// native bench has generated the folder (docs/spikes/QA-7-banc-de-charge-format-projet.md):
//
//   FAULK_BENCH_DIR=<folder> FAULK_BENCH_WASM=<load_wasm.wasm> pnpm bench:project
//
// The folder is served from a loopback server, fetched into the page, then written to and read
// from OPFS under the three ways a store can use it (async writable, a worker's synchronous
// access handle, several workers). The Rust reader runs in the page as the product's will
// (wasm32), fed from OPFS. The real folder (File System Access, `showDirectoryPicker`) needs a
// native dialog and a user gesture: Playwright cannot drive it, so it is not measured here.

const DIR = process.env.FAULK_BENCH_DIR ?? ''
const WASM = process.env.FAULK_BENCH_WASM ?? ''

type Entry = { path: string; size: number }

function walk(root: string, skip: (rel: string) => boolean): Entry[] {
  const out: Entry[] = []
  const visit = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const full = join(dir, name)
      const rel = relative(root, full).split(sep).join('/')
      if (skip(rel)) continue
      const s = statSync(full)
      if (s.isDirectory()) visit(full)
      else out.push({ path: rel, size: s.size })
    }
  }
  visit(root)
  return out.sort((a, b) => (a.path < b.path ? -1 : 1))
}

function serve(files: Entry[]): Promise<{ server: Server; origin: string }> {
  const server = createServer((req, res) => {
    const url = decodeURIComponent((req.url ?? '/').split('?')[0])
    const send = (body: Buffer | string, type: string) => {
      res.writeHead(200, { 'content-type': type, 'cache-control': 'no-store' })
      res.end(body)
    }
    if (url === '/') return send('<!doctype html><title>qa7</title>', 'text/html')
    if (url === '/manifest.json') return send(JSON.stringify(files), 'application/json')
    if (url === '/load.wasm') return send(readFileSync(WASM), 'application/wasm')
    if (url === '/snapshot') return send(readFileSync(join(DIR, '.faulk/cache/project.snap')), 'application/octet-stream')
    if (url.startsWith('/f/')) {
      const rel = url.slice(3)
      if (rel.includes('..')) {
        res.writeHead(400).end()
        return
      }
      return send(readFileSync(join(DIR, rel)), 'application/octet-stream')
    }
    res.writeHead(404).end()
  })
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () => resolve({ server, origin: `http://localhost:${(server.address() as { port: number }).port}` })),
  )
}

/** Helpers the measurements share, defined once in the page. */
async function defineHelpers(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as Record<string, unknown>
    const now = () => performance.now()
    const root = () => navigator.storage.getDirectory()
    const median = (v: number[]) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)]
    const dirCache = new Map<string, FileSystemDirectoryHandle>()
    async function dir(base: FileSystemDirectoryHandle, baseKey: string, parts: string[]) {
      let cur = base
      let key = baseKey
      for (const p of parts) {
        key += '/' + p
        let next = dirCache.get(key)
        if (!next) {
          next = await cur.getDirectoryHandle(p, { create: true })
          dirCache.set(key, next)
        }
        cur = next
      }
      return cur
    }
    async function fresh(name: string) {
      const r = await root()
      try {
        await r.removeEntry(name, { recursive: true })
      } catch {
        /* absent */
      }
      for (const k of [...dirCache.keys()]) if (k.startsWith(name)) dirCache.delete(k)
      return r.getDirectoryHandle(name, { create: true })
    }
    async function pool<T>(n: number, items: T[], fn: (item: T) => Promise<void>) {
      let next = 0
      await Promise.all(
        Array.from({ length: n }, async () => {
          while (next < items.length) await fn(items[next++])
        }),
      )
    }
    async function listAll(handle: FileSystemDirectoryHandle, prefix = ''): Promise<string[]> {
      const out: string[] = []
      const entries = (handle as unknown as { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries()
      for await (const [name, h] of entries) {
        if (h.kind === 'directory') out.push(...(await listAll(h as FileSystemDirectoryHandle, prefix + name + '/')))
        else out.push(prefix + name)
      }
      return out
    }
    Object.assign(w, { __qa7: { now, root, median, dir, fresh, pool, listAll } })
  })
}

const SYNC_WORKER = `
const dirs = new Map();
async function dir(base, key, parts) {
  let cur = base;
  for (const p of parts) {
    key += '/' + p;
    let next = dirs.get(key);
    if (!next) { next = await cur.getDirectoryHandle(p, { create: true }); dirs.set(key, next); }
    cur = next;
  }
  return cur;
}
self.onmessage = async (e) => {
  const m = e.data;
  const base = await navigator.storage.getDirectory();
  if (m.type === 'write') {
    const top = await base.getDirectoryHandle(m.name, { create: true });
    const t0 = performance.now();
    for (const [path, buf] of m.files) {
      const parts = path.split('/');
      const file = parts.pop();
      const d = await dir(top, m.name, parts);
      const fh = await d.getFileHandle(file, { create: true });
      const ah = await fh.createSyncAccessHandle();
      ah.truncate(0);
      ah.write(new Uint8Array(buf), { at: 0 });
      ah.flush();
      ah.close();
    }
    self.postMessage({ type: 'done', ms: performance.now() - t0 });
  } else if (m.type === 'read') {
    const top = await base.getDirectoryHandle(m.name);
    const t0 = performance.now();
    let bytes = 0;
    const out = [];
    for (const path of m.paths) {
      const parts = path.split('/');
      const file = parts.pop();
      const d = await dir(top, m.name, parts);
      const fh = await d.getFileHandle(file);
      const ah = await fh.createSyncAccessHandle();
      const buf = new Uint8Array(ah.getSize());
      ah.read(buf, { at: 0 });
      ah.close();
      bytes += buf.length;
      if (m.keep) out.push(buf.buffer);
    }
    self.postMessage({ type: 'done', ms: performance.now() - t0, bytes, buffers: out }, out);
  } else if (m.type === 'rewrite') {
    const top = await base.getDirectoryHandle(m.name);
    const parts = m.path.split('/');
    const file = parts.pop();
    const d = await dir(top, m.name, parts);
    const times = [];
    for (let i = 0; i < m.runs; i++) {
      const t0 = performance.now();
      const fh = await d.getFileHandle(file);
      const ah = await fh.createSyncAccessHandle();
      ah.truncate(0);
      ah.write(new Uint8Array(m.buf), { at: 0 });
      ah.flush();
      ah.close();
      times.push(performance.now() - t0);
    }
    self.postMessage({ type: 'done', times });
  }
};
`

test.describe.configure({ mode: 'serial' })

test.describe('QA-7: the project folder in OPFS and the Rust reader in the page', () => {
  test.skip(!DIR || !WASM || !existsSync(DIR) || !existsSync(WASM), 'set FAULK_BENCH_DIR and FAULK_BENCH_WASM (see the header of this file)')

  let origin = ''
  let closeServer: () => void = () => {}
  let files: Entry[] = []

  test.beforeAll(async () => {
    // The text of the project and its cache snapshot are measured here; the audio blobs and git
    // are measured natively, and the audio's cost in the browser by its own test below.
    files = walk(DIR, (rel) => rel === 'blobs' || rel === '.faulk' || rel === '.git' || rel.endsWith('.tmp'))
    const s = await serve(files)
    origin = s.origin
    closeServer = () => s.server.close()
  })
  test.afterAll(() => closeServer())

  test('write, list and read the folder in OPFS, then open it with the wasm reader', async ({ page }) => {
    test.setTimeout(30 * 60_000)
    const errors: string[] = []
    page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
    page.on('pageerror', (e) => errors.push(String(e)))
    await page.goto(origin + '/')
    await defineHelpers(page)
    const total = files.reduce((n, f) => n + f.size, 0)
    console.log(`QA-7 browser: ${files.length} files, ${(total / 1048576).toFixed(1)} MiB of text; ${await page.evaluate(() => navigator.userAgent)}`)
    const quota = await page.evaluate(async () => {
      const e = await navigator.storage.estimate()
      return { quota: e.quota ?? 0, usage: e.usage ?? 0 }
    })
    console.log(`storage quota ${(quota.quota / 1073741824).toFixed(1)} GiB, used ${(quota.usage / 1048576).toFixed(1)} MiB`)

    // Fetch every file into memory (loopback), once.
    const fetched = await page.evaluate(async (list: Entry[]) => {
      const q = (window as unknown as { __qa7: { now(): number; pool: (n: number, items: Entry[], f: (e: Entry) => Promise<void>) => Promise<void> } }).__qa7
      const store = new Map<string, ArrayBuffer>()
      ;(window as unknown as { __files: Map<string, ArrayBuffer> }).__files = store
      const t0 = q.now()
      await q.pool(16, list, async (e) => {
        const r = await fetch('/f/' + e.path)
        store.set(e.path, await r.arrayBuffer())
      })
      return { ms: q.now() - t0, count: store.size }
    }, files)
    console.log(`fetched ${fetched.count} files from the loopback server into memory: ${fetched.ms.toFixed(0)} ms (not part of any measure below)`)
    expect(fetched.count).toBe(files.length)

    // Writes through the async API (main thread), 1 / 8 / 32 at a time.
    for (const concurrency of [1, 8, 32]) {
      const runs = concurrency === 1 ? 2 : 3
      const times = await page.evaluate(
        async ({ concurrency, runs }) => {
          const q = (window as unknown as { __qa7: Record<string, (...a: never[]) => unknown> }).__qa7 as unknown as {
            now(): number
            fresh(n: string): Promise<FileSystemDirectoryHandle>
            dir(b: FileSystemDirectoryHandle, k: string, p: string[]): Promise<FileSystemDirectoryHandle>
            pool<T>(n: number, items: T[], f: (i: T) => Promise<void>): Promise<void>
          }
          const store = (window as unknown as { __files: Map<string, ArrayBuffer> }).__files
          const out: number[] = []
          for (let r = 0; r < runs; r++) {
            const top = await q.fresh('qa7-async')
            const items = [...store.entries()]
            const t0 = q.now()
            await q.pool(concurrency, items, async ([path, buf]) => {
              const parts = path.split('/')
              const name = parts.pop() as string
              const d = await q.dir(top, 'qa7-async', parts)
              const fh = await d.getFileHandle(name, { create: true })
              const w = await fh.createWritable()
              await w.write(buf)
              await w.close()
            })
            out.push(q.now() - t0)
          }
          return out
        },
        { concurrency, runs },
      )
      console.log(
        `OPFS write of the whole folder, async createWritable, ${concurrency} at a time: ${median(times).toFixed(0)} ms median (runs ${times.map((t) => t.toFixed(0)).join(', ')})`,
      )
    }

    // One worker, synchronous access handles.
    {
      const times = await page.evaluate(async (src) => {
        const store = (window as unknown as { __files: Map<string, ArrayBuffer> }).__files
        const out: number[] = []
        for (let r = 0; r < 3; r++) {
          const worker = new Worker(URL.createObjectURL(new Blob([src], { type: 'text/javascript' })))
          await (async () => {
            const root = await navigator.storage.getDirectory()
            try {
              await root.removeEntry('qa7-sync', { recursive: true })
            } catch {
              /* absent */
            }
          })()
          const copy = [...store.entries()].map(([p, b]) => [p, b.slice(0)] as [string, ArrayBuffer])
          const ms = await new Promise<number>((resolve) => {
            worker.onmessage = (e) => resolve(e.data.ms)
            worker.postMessage({ type: 'write', name: 'qa7-sync', files: copy })
          })
          out.push(ms)
          worker.terminate()
        }
        return out
      }, SYNC_WORKER)
      console.log(
        `OPFS write of the whole folder, one worker, synchronous access handles: ${median(times).toFixed(0)} ms median (runs ${times.map((t) => t.toFixed(0)).join(', ')})`,
      )
    }

    // Listing, then reads: async 1/8/32, then a worker's sync handles.
    const listing = await page.evaluate(async () => {
      const q = (
        window as unknown as { __qa7: { now(): number; root(): Promise<FileSystemDirectoryHandle>; listAll(h: FileSystemDirectoryHandle): Promise<string[]> } }
      ).__qa7
      const top = await (await q.root()).getDirectoryHandle('qa7-async')
      const out: number[] = []
      let n = 0
      for (let i = 0; i < 5; i++) {
        const t0 = q.now()
        n = (await q.listAll(top)).length
        out.push(q.now() - t0)
      }
      return { out, n }
    })
    console.log(`OPFS listing of the whole tree (entries()): ${median(listing.out).toFixed(0)} ms median over 5, ${listing.n} files`)
    for (const concurrency of [1, 8, 32]) {
      const times = await page.evaluate(async (concurrency) => {
        const q = (
          window as unknown as {
            __qa7: {
              now(): number
              root(): Promise<FileSystemDirectoryHandle>
              dir(b: FileSystemDirectoryHandle, k: string, p: string[]): Promise<FileSystemDirectoryHandle>
              pool<T>(n: number, items: T[], f: (i: T) => Promise<void>): Promise<void>
            }
          }
        ).__qa7
        const top = await (await q.root()).getDirectoryHandle('qa7-async')
        const paths = [...(window as unknown as { __files: Map<string, ArrayBuffer> }).__files.keys()]
        const out: number[] = []
        for (let r = 0; r < 3; r++) {
          let bytes = 0
          const t0 = q.now()
          await q.pool(concurrency, paths, async (path) => {
            const parts = path.split('/')
            const name = parts.pop() as string
            const d = await q.dir(top, 'qa7-async', parts)
            const f = await (await d.getFileHandle(name)).getFile()
            bytes += (await f.arrayBuffer()).byteLength
          })
          out.push(q.now() - t0)
          if (bytes === 0) throw new Error('nothing read')
        }
        return out
      }, concurrency)
      console.log(
        `OPFS read of the whole folder, async getFile().arrayBuffer(), ${concurrency} at a time: ${median(times).toFixed(0)} ms median (runs ${times.map((t) => t.toFixed(0)).join(', ')})`,
      )
    }
    {
      const times = await page.evaluate(
        async ({ src, paths }) => {
          const out: number[] = []
          for (let r = 0; r < 3; r++) {
            const worker = new Worker(URL.createObjectURL(new Blob([src], { type: 'text/javascript' })))
            const ms = await new Promise<number>((resolve) => {
              worker.onmessage = (e) => resolve(e.data.ms)
              worker.postMessage({ type: 'read', name: 'qa7-sync', paths, keep: false })
            })
            out.push(ms)
            worker.terminate()
          }
          return out
        },
        { src: SYNC_WORKER, paths: files.map((f) => f.path) },
      )
      console.log(
        `OPFS read of the whole folder, one worker, synchronous access handles: ${median(times).toFixed(0)} ms median (runs ${times.map((t) => t.toFixed(0)).join(', ')})`,
      )
    }

    // The save of one note: the one clip file rewritten.
    const clip = files.find((f) => f.path.endsWith('.notes.tsv')) as Entry
    {
      const res = await page.evaluate(
        async ({ src, path }) => {
          const q = (
            window as unknown as {
              __qa7: {
                now(): number
                root(): Promise<FileSystemDirectoryHandle>
                dir(b: FileSystemDirectoryHandle, k: string, p: string[]): Promise<FileSystemDirectoryHandle>
              }
            }
          ).__qa7
          const store = (window as unknown as { __files: Map<string, ArrayBuffer> }).__files
          const buf = store.get(path) as ArrayBuffer
          const top = await (await q.root()).getDirectoryHandle('qa7-async')
          const parts = path.split('/')
          const name = parts.pop() as string
          const d = await q.dir(top, 'qa7-async', parts)
          const asyncTimes: number[] = []
          for (let i = 0; i < 50; i++) {
            const t0 = q.now()
            const fh = await d.getFileHandle(name)
            const w = await fh.createWritable()
            await w.write(buf)
            await w.close()
            asyncTimes.push(q.now() - t0)
          }
          const worker = new Worker(URL.createObjectURL(new Blob([src], { type: 'text/javascript' })))
          const syncTimes = await new Promise<number[]>((resolve) => {
            worker.onmessage = (e) => resolve(e.data.times)
            worker.postMessage({ type: 'rewrite', name: 'qa7-sync', path, buf, runs: 50 })
          })
          worker.terminate()
          return { asyncTimes, syncTimes, size: buf.byteLength }
        },
        { src: SYNC_WORKER, path: clip.path },
      )
      const med = (v: number[]) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)]
      const p95 = (v: number[]) => [...v].sort((a, b) => a - b)[Math.floor(v.length * 0.95)]
      console.log(
        `save of one note (one ${res.size}-byte clip file rewritten), async createWritable: median ${med(res.asyncTimes).toFixed(2)} ms, p95 ${p95(res.asyncTimes).toFixed(2)} ms (n=50)`,
      )
      console.log(
        `save of one note, synchronous access handle in a worker: median ${med(res.syncTimes).toFixed(2)} ms, p95 ${p95(res.syncTimes).toFixed(2)} ms (n=50)`,
      )
    }

    // The cache snapshot as one file: write, read.
    const snap = await page.evaluate(async () => {
      const q = (window as unknown as { __qa7: { now(): number; root(): Promise<FileSystemDirectoryHandle> } }).__qa7
      const r = await fetch('/snapshot')
      const buf = await r.arrayBuffer()
      ;(window as unknown as { __snap: ArrayBuffer }).__snap = buf
      const root = await q.root()
      const writes: number[] = []
      const reads: number[] = []
      for (let i = 0; i < 3; i++) {
        let t0 = q.now()
        const fh = await root.getFileHandle('qa7-snapshot', { create: true })
        const w = await fh.createWritable()
        await w.write(buf)
        await w.close()
        writes.push(q.now() - t0)
        t0 = q.now()
        const back = await (await fh.getFile()).arrayBuffer()
        reads.push(q.now() - t0)
        if (back.byteLength !== buf.byteLength) throw new Error('snapshot size')
      }
      return { size: buf.byteLength, writes, reads }
    })
    console.log(
      `cache snapshot (${(snap.size / 1048576).toFixed(1)} MiB, one file): write ${median(snap.writes).toFixed(0)} ms, read ${median(snap.reads).toFixed(0)} ms (median of 3)`,
    )

    // The Rust reader, in the page: open from the OPFS files, then from the snapshot.
    const open = await page.evaluate(
      async ({ list }) => {
        const q = (
          window as unknown as {
            __qa7: {
              now(): number
              root(): Promise<FileSystemDirectoryHandle>
              dir(b: FileSystemDirectoryHandle, k: string, p: string[]): Promise<FileSystemDirectoryHandle>
              pool<T>(n: number, items: T[], f: (i: T) => Promise<void>): Promise<void>
            }
          }
        ).__qa7
        type Ex = {
          memory: WebAssembly.Memory
          fk_bench_alloc(n: number): number
          fk_bench_put(pp: number, pl: number, dp: number, dl: number): void
          fk_bench_drop_files(): void
          fk_bench_drop_project(): void
          fk_bench_open(): number
          fk_bench_open_snapshot(p: number, l: number): number
          fk_bench_validate(): number
          fk_bench_serialize_all(): number
          fk_bench_serialize_one_clip(): number
          fk_bench_notes(): number
          fk_bench_points(): number
          fk_bench_memory(): number
          fk_bench_error_len(): number
          fk_bench_error_ptr(): number
        }
        const bytes = await (await fetch('/load.wasm')).arrayBuffer()
        const heap = () => (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? 0
        const results: Record<string, number | string> = { wasmBytes: bytes.byteLength }
        const top = await (await q.root()).getDirectoryHandle('qa7-async')
        const enc = new TextEncoder()
        const err = (ex: Ex) => new TextDecoder().decode(new Uint8Array(ex.memory.buffer, ex.fk_bench_error_ptr(), ex.fk_bench_error_len()))

        const runOnce = async (from: 'files' | 'snapshot') => {
          const { instance } = await WebAssembly.instantiate(bytes, {})
          const ex = instance.exports as unknown as Ex
          const out: Record<string, number | string> = {}
          const t0 = q.now()
          let read: number
          let copy: number
          let code: number
          if (from === 'files') {
            const bufs: [string, ArrayBuffer][] = []
            let t = q.now()
            await q.pool(8, list, async (e: Entry) => {
              const parts = e.path.split('/')
              const name = parts.pop() as string
              const d = await q.dir(top, 'qa7-async', parts)
              const f = await (await d.getFileHandle(name)).getFile()
              bufs.push([e.path, await f.arrayBuffer()])
            })
            read = q.now() - t
            t = q.now()
            for (const [path, buf] of bufs) {
              const pb = enc.encode(path)
              const pp = ex.fk_bench_alloc(pb.length)
              new Uint8Array(ex.memory.buffer, pp, pb.length).set(pb)
              const dp = ex.fk_bench_alloc(buf.byteLength)
              new Uint8Array(ex.memory.buffer, dp, buf.byteLength).set(new Uint8Array(buf))
              ex.fk_bench_put(pp, pb.length, dp, buf.byteLength)
            }
            copy = q.now() - t
            out.peakAfterCopy = ex.fk_bench_memory()
            t = q.now()
            code = ex.fk_bench_open()
            out.parse = q.now() - t
          } else {
            // The validity check the snapshot needs: every source file's size and date, as the
            // native check stats them (32 at a time).
            let t = q.now()
            let stale = 0
            await q.pool(32, list, async (e: Entry) => {
              const parts = e.path.split('/')
              const name = parts.pop() as string
              const d = await q.dir(top, 'qa7-async', parts)
              const info = await (await d.getFileHandle(name)).getFile()
              if (info.size !== e.size) stale++
              void info.lastModified
            })
            out.check = q.now() - t
            if (stale > 0) throw new Error(`${stale} files differ in size`)
            t = q.now()
            const f = await (await (await q.root()).getFileHandle('qa7-snapshot')).getFile()
            const buf = await f.arrayBuffer()
            read = q.now() - t
            t = q.now()
            const dp = ex.fk_bench_alloc(buf.byteLength)
            new Uint8Array(ex.memory.buffer, dp, buf.byteLength).set(new Uint8Array(buf))
            copy = q.now() - t
            out.peakAfterCopy = ex.fk_bench_memory()
            t = q.now()
            code = ex.fk_bench_open_snapshot(dp, buf.byteLength)
            out.parse = q.now() - t
          }
          if (code < 0) throw new Error(`open failed: ${err(ex)}`)
          out.read = read
          out.copy = copy
          out.tracks = code
          out.total = q.now() - t0
          out.notes = ex.fk_bench_notes()
          out.points = ex.fk_bench_points()
          let t = q.now()
          out.violations = ex.fk_bench_validate()
          out.validate = q.now() - t
          ex.fk_bench_drop_files()
          out.memory = ex.fk_bench_memory()
          out.jsHeap = heap()
          t = q.now()
          out.serializeBytes = ex.fk_bench_serialize_all()
          out.serialize = q.now() - t
          t = q.now()
          const clipBytes = ex.fk_bench_serialize_one_clip()
          out.serializeOneClip = q.now() - t
          out.clipBytes = clipBytes
          ex.fk_bench_drop_project()
          return out
        }
        const runs: Record<string, Record<string, number | string>[]> = { files: [], snapshot: [] }
        for (const from of ['files', 'snapshot'] as const) for (let i = 0; i < 3; i++) runs[from].push(await runOnce(from))
        return { results, runs }
      },
      { list: files },
    )
    for (const from of ['files', 'snapshot'] as const) {
      const runs = open.runs[from]
      const pick = (k: string) => median(runs.map((r) => Number(r[k])))
      const last = runs[runs.length - 1]
      console.log(
        `open in the page from ${from === 'files' ? 'the OPFS files (8 at a time)' : 'the cache snapshot (one file)'}: total ${pick('total').toFixed(0)} ms median of 3 = ${from === 'snapshot' ? `validity check of every file's size and date ${pick('check').toFixed(0)} + ` : ''}read ${pick('read').toFixed(0)} + copy into wasm ${pick('copy').toFixed(0)} + Rust reader ${pick('parse').toFixed(0)} (runs ${runs.map((r) => Number(r.total).toFixed(0)).join(', ')})`,
      )
      console.log(
        `  ${last.tracks} tracks, ${last.notes} notes, ${last.points} points; validate ${pick('validate').toFixed(0)} ms (${last.violations} violations); wasm memory ${(Number(last.memory) / 1048576).toFixed(0)} MiB after the open (${(Number(last.peakAfterCopy) / 1048576).toFixed(0)} MiB when the input was copied in); JS heap ${(Number(last.jsHeap) / 1048576).toFixed(0)} MiB`,
      )
      console.log(
        `  serialise every file of the project (no write): ${pick('serialize').toFixed(0)} ms, ${(Number(last.serializeBytes) / 1048576).toFixed(1)} MiB; one clip's notes file: ${pick('serializeOneClip').toFixed(2)} ms (${last.clipBytes} bytes)`,
      )
      expect(last.notes).toBe(200000)
      expect(last.points).toBe(6000000)
      expect(last.violations).toBe(0)
    }
    expect(errors).toEqual([])
  })

  test('a flat blobs/ folder against 256 sub-folders in OPFS', async ({ page }) => {
    test.setTimeout(30 * 60_000)
    await page.goto(origin + '/')
    await defineHelpers(page)
    for (const n of [1000, 10000]) {
      for (const fan of [false, true]) {
        const r = await page.evaluate(
          async ({ n, fan }) => {
            const q = (
              window as unknown as {
                __qa7: { now(): number; fresh(n: string): Promise<FileSystemDirectoryHandle>; listAll(h: FileSystemDirectoryHandle): Promise<string[]> }
              }
            ).__qa7
            const top = await q.fresh('qa7-blobs')
            const hex = (i: number) => {
              // A SHA-256 look-alike: 64 hex characters from the index, spread over the first byte.
              let x = (i + 1) * 2654435761
              let s = ''
              for (let k = 0; k < 8; k++) {
                x = (Math.imul(x ^ (x >>> 15), 2246822519) + k) >>> 0
                s += x.toString(16).padStart(8, '0')
              }
              return s
            }
            const names = Array.from({ length: n }, (_, i) => hex(i))
            const subs = new Map<string, FileSystemDirectoryHandle>()
            const payload = new Uint8Array(64)
            const t0 = q.now()
            for (const name of names) {
              let dir = top
              if (fan) {
                const key = name.slice(0, 2)
                let d = subs.get(key)
                if (!d) {
                  d = await top.getDirectoryHandle(key, { create: true })
                  subs.set(key, d)
                }
                dir = d
              }
              const fh = await dir.getFileHandle(name, { create: true })
              const w = await fh.createWritable()
              await w.write(payload)
              await w.close()
            }
            const create = q.now() - t0
            const t1 = q.now()
            const listed = (await q.listAll(top)).length
            const list = q.now() - t1
            const t2 = q.now()
            for (let i = 0; i < 200; i++) {
              const name = names[(i * 7919) % n]
              const dir = fan ? await top.getDirectoryHandle(name.slice(0, 2)) : top
              await (await dir.getFileHandle(name)).getFile()
            }
            const look = q.now() - t2
            return { create, list, listed, look }
          },
          { n, fan },
        )
        console.log(
          `OPFS blobs/ ${fan ? '256 sub-folders' : 'flat'}, ${n} files of 64 bytes: create ${r.create.toFixed(0)} ms (${((r.create / n) * 1000).toFixed(0)} µs per file), list ${r.list.toFixed(0)} ms, 200 lookups ${r.look.toFixed(0)} ms (listed ${r.listed})`,
        )
        expect(r.listed).toBe(n)
      }
    }
  })

  test('the project model at CT-3 safety limits in the page (wasm32)', async ({ page }) => {
    test.setTimeout(30 * 60_000)
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(String(e)))
    await page.goto(origin + '/')
    const r = await page.evaluate(async () => {
      type Ex = {
        memory: WebAssembly.Memory
        fk_bench_generate_limits(): number
        fk_bench_validate(): number
        fk_bench_notes(): number
        fk_bench_points(): number
        fk_bench_memory(): number
        fk_bench_serialize_one_clip(): number
      }
      const bytes = await (await fetch('/load.wasm')).arrayBuffer()
      const { instance } = await WebAssembly.instantiate(bytes, {})
      const ex = instance.exports as unknown as Ex
      const before = ex.fk_bench_memory()
      let t = performance.now()
      const tracks = ex.fk_bench_generate_limits()
      const generate = performance.now() - t
      const after = ex.fk_bench_memory()
      t = performance.now()
      const violations = ex.fk_bench_validate()
      const validate = performance.now() - t
      return { tracks, generate, validate, violations, notes: ex.fk_bench_notes(), points: ex.fk_bench_points(), before, after }
    })
    console.log(
      `project at the safety limits in wasm32: ${r.tracks} tracks, ${r.notes} notes, ${r.points} points built in ${r.generate.toFixed(0)} ms, validated in ${r.validate.toFixed(0)} ms (${r.violations} violations); linear memory ${(r.before / 1048576).toFixed(0)} MiB at start, ${(r.after / 1048576).toFixed(0)} MiB after (cap 4096 MiB)`,
    )
    expect(r.notes).toBe(17 * 986_895)
    expect(r.points).toBe(17 * 986_895)
    expect(r.violations).toBe(0)
    expect(errors).toEqual([])
  })

  test('an audio blob in OPFS: write, read, SHA-256', async ({ page }) => {
    test.setTimeout(30 * 60_000)
    await page.goto(origin + '/')
    await defineHelpers(page)
    const r = await page.evaluate(async () => {
      const q = (window as unknown as { __qa7: { now(): number; fresh(n: string): Promise<FileSystemDirectoryHandle> } }).__qa7
      const top = await q.fresh('qa7-audio')
      const chunk = new Uint8Array(4 * 1048576)
      for (let i = 0; i < chunk.length; i += 65536) crypto.getRandomValues(chunk.subarray(i, i + 65536))
      const out: { mib: number; write: number; read: number; sha: number }[] = []
      for (const mib of [64, 256]) {
        const fh = await top.getFileHandle(`blob-${mib}`, { create: true })
        let t0 = q.now()
        const w = await fh.createWritable()
        for (let i = 0; i < mib / 4; i++) await w.write(chunk)
        await w.close()
        const write = q.now() - t0
        t0 = q.now()
        const buf = await (await fh.getFile()).arrayBuffer()
        const read = q.now() - t0
        t0 = q.now()
        await crypto.subtle.digest('SHA-256', buf)
        const sha = q.now() - t0
        out.push({ mib, write, read, sha })
      }
      const e = await navigator.storage.estimate()
      return { out, usage: e.usage ?? 0, quota: e.quota ?? 0 }
    })
    for (const o of r.out) {
      console.log(
        `audio blob ${o.mib} MiB in OPFS: write ${o.write.toFixed(0)} ms (${((o.mib / o.write) * 1000).toFixed(0)} MiB/s), read back ${o.read.toFixed(0)} ms, SubtleCrypto SHA-256 of the whole buffer ${o.sha.toFixed(0)} ms`,
      )
    }
    console.log(`storage used ${(r.usage / 1048576).toFixed(0)} MiB of ${(r.quota / 1073741824).toFixed(1)} GiB`)
  })
})

function median(v: number[]) {
  return [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)]
}
