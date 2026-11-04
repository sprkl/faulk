// The disk side of the Web engine (AU-3): the executor's job table, paths and failures against a
// fake folder, then the shipped engine module under Node, two instances on one shared memory as
// the prepare Worker and the AudioWorklet run it: the prepare instance plans the reads and the
// executor serves them from a folder in memory, the render instance's test voice reads the rings
// and the resident segments (the consumer until AU-4's clip player). The control stream is the
// Rust scenario's (`crates/engine-wasm/tests/fixtures/stream.stream`), its sources made here
// with the same formulas as `engine::audio_source::scenario`.
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { becomeRenderInstance, createChannel, instantiate, ROLE_RENDER } from '../../engine/engine-abi'
import { readFeedback } from '../../engine/feedback'
import { DiskExecutor, FAIL_CHANGED, FAIL_NOT_FOUND, JOB_BYTES, pathOf, readJobs, type Dir, type DiskExports } from './executor'

const RATE = 8_000
const BLOCK = 128
const LONG_FRAMES = 40 * RATE
const SHORT_FRAMES = 10 * RATE
const CLIP_START = 2 * RATE
const LONG = 'a1'.repeat(32)
const SHORT = 'b2'.repeat(32)

const longSample = (f: number, c: number) => ((f * 7 + c * 3) % 60_000) - 30_000
const longValue = (f: number, c: number) => Math.fround(longSample(f, c) / 32_768)
const shortValue = (f: number, c: number) => Math.fround(Math.fround(f / 1_000_000) + c)

function longWav(): Uint8Array {
  const data = LONG_FRAMES * 4
  const out = new Uint8Array(44 + data)
  const v = new DataView(out.buffer)
  const ascii = (at: number, s: string) => [...s].forEach((ch, i) => (out[at + i] = ch.charCodeAt(0)))
  ascii(0, 'RIFF')
  v.setUint32(4, 36 + data, true)
  ascii(8, 'WAVEfmt ')
  v.setUint32(16, 16, true)
  v.setUint16(20, 1, true)
  v.setUint16(22, 2, true)
  v.setUint32(24, RATE, true)
  v.setUint32(28, RATE * 4, true)
  v.setUint16(32, 4, true)
  v.setUint16(34, 16, true)
  ascii(36, 'data')
  v.setUint32(40, data, true)
  for (let f = 0; f < LONG_FRAMES; f++) for (let c = 0; c < 2; c++) v.setInt16(44 + f * 4 + c * 2, longSample(f, c), true)
  return out
}

function shortCache(): Uint8Array {
  const out = new Uint8Array(32 + SHORT_FRAMES * 8)
  const v = new DataView(out.buffer)
  ;[...'FKPCM001'].forEach((ch, i) => (out[i] = ch.charCodeAt(0)))
  v.setUint32(8, RATE, true)
  v.setUint16(12, 2, true)
  v.setBigUint64(16, BigInt(SHORT_FRAMES), true)
  v.setUint32(24, 1, true)
  for (let f = 0; f < SHORT_FRAMES; f++) for (let c = 0; c < 2; c++) v.setFloat32(32 + f * 8 + c * 4, shortValue(f, c), true)
  return out
}

/** A folder held in memory: paths to bytes; `broken` paths throw that error's name when read. */
function memoryDir(files: Map<string, Uint8Array>, broken = new Map<string, string>()): Dir {
  const at = (prefix: string): Dir => ({
    async getDirectoryHandle(name) {
      return at(`${prefix}${name}/`)
    },
    async getFileHandle(name) {
      const path = `${prefix}${name}`
      const bytes = files.get(path)
      if (!bytes) throw new DOMException(path, 'NotFoundError')
      return {
        async getFile() {
          const error = broken.get(path)
          if (error) {
            const blob = new Blob([bytes.slice()])
            // A `File` whose reads fail as the browser's do when the file changed under it.
            return Object.assign(blob, {
              slice: () => ({
                arrayBuffer: () => Promise.reject(new DOMException(path, error)),
              }),
            }) as unknown as File
          }
          return new File([bytes.slice()], name)
        },
      }
    },
  })
  return at('')
}

/** Fake exports of one job, to drive the executor alone. */
function fakeExports(job: { id: number; file: 0 | 1; media: string; offset: number; len: number }) {
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true })
  const table = new DataView(memory.buffer, 0, JOB_BYTES)
  table.setUint32(0, job.id, true)
  table.setUint8(4, job.file)
  table.setBigUint64(8, BigInt(job.offset), true)
  table.setUint32(16, job.len, true)
  for (let i = 0; i < 32; i++) table.setUint8(24 + i, parseInt(job.media.slice(2 * i, 2 * i + 2), 16))
  const calls: { done: [number, number, number][]; failed: [number, number][] } = { done: [], failed: [] }
  let polled = false
  const exports: DiskExports = {
    fk_disk_poll: () => (polled ? 0 : ((polled = true), 1)),
    fk_disk_jobs: () => 0,
    fk_disk_inbox: () => 1024,
    fk_disk_done: (_ch, id, len, size) => (calls.done.push([id, len, size]), 0),
    fk_disk_failed: (_ch, id, code) => void calls.failed.push([id, code]),
  }
  return { memory, exports, calls }
}

describe('the disk executor', () => {
  it('reads a job table and names the files of a media', () => {
    const { memory } = fakeExports({ id: 7, file: 1, media: SHORT, offset: 2 ** 40, len: 4096 })
    expect(readJobs(memory.buffer, 0, 1)).toEqual([{ id: 7, file: 1, media: SHORT, offset: 2 ** 40, len: 4096 }])
    expect(pathOf({ file: 0, media: LONG })).toEqual(['blobs', LONG])
    expect(pathOf({ file: 1, media: LONG })).toEqual(['.faulk', 'cache', 'pcm', `${LONG}.pcm`])
  })

  it('answers a range of the file, with the file size', async () => {
    const { memory, exports, calls } = fakeExports({ id: 3, file: 0, media: LONG, offset: 4, len: 8 })
    const disk = new DiskExecutor(exports, memory, 1)
    disk.folder(memoryDir(new Map([[`blobs/${LONG}`, new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13])]])))
    disk.tick()
    await Promise.all(disk.inFlight)
    expect(calls.done).toEqual([[3, 8, 14]])
    expect([...new Uint8Array(memory.buffer, 1024, 8)]).toEqual([4, 5, 6, 7, 8, 9, 10, 11])
  })

  it('says not found without a folder or a file, and changed when the file changed under it', async () => {
    const run = async (dir: Dir | null) => {
      const { memory, exports, calls } = fakeExports({ id: 1, file: 1, media: LONG, offset: 0, len: 32 })
      const disk = new DiskExecutor(exports, memory, 1)
      disk.folder(dir)
      disk.tick()
      await Promise.all(disk.inFlight)
      return calls.failed
    }
    expect(await run(null)).toEqual([[1, FAIL_NOT_FOUND]])
    expect(await run(memoryDir(new Map()))).toEqual([[1, FAIL_NOT_FOUND]])
    const path = `.faulk/cache/pcm/${LONG}.pcm`
    expect(await run(memoryDir(new Map([[path, new Uint8Array(64)]]), new Map([[path, 'NotReadableError']])))).toEqual([[1, FAIL_CHANGED]])
  })

  it('answers a read of a folder it no longer has as not found', async () => {
    const { memory, exports, calls } = fakeExports({ id: 2, file: 0, media: LONG, offset: 0, len: 4 })
    const disk = new DiskExecutor(exports, memory, 1)
    disk.folder(memoryDir(new Map([[`blobs/${LONG}`, new Uint8Array(16)]])))
    disk.tick()
    disk.folder(null)
    await Promise.all(disk.inFlight)
    expect(calls.failed).toEqual([[2, FAIL_NOT_FOUND]])
  })
})

const bytes = new Uint8Array(readFileSync(new URL('../../engine/wasm/engine.wasm', import.meta.url))).buffer
const module = new WebAssembly.Module(bytes)
const fixture = new Uint8Array(readFileSync(new URL('../../../../../crates/engine-wasm/tests/fixtures/stream.stream', import.meta.url)))

describe('the shipped engine module streams from a folder', () => {
  function rig(files: Map<string, Uint8Array>) {
    const memory = new WebAssembly.Memory({ initial: 256, maximum: 65_536, shared: true })
    const prepare = instantiate(module, memory)
    const setup = createChannel(prepare, RATE)
    const render = instantiate(module, memory)
    becomeRenderInstance(render, setup)
    const ch = setup.channel
    const at = prepare.fk_inbox(ch, fixture.length)
    new Uint8Array(memory.buffer, at, fixture.length).set(fixture)
    expect(prepare.fk_push(ch, at, fixture.length)).toBe(0)
    let clock = 0
    const disk = new DiskExecutor(prepare, memory, ch, () => clock)
    disk.folder(memoryDir(files))
    /** One block: the worklet's pump and voice, then the Worker's flush and disk tick. */
    const block = async (read: boolean) => {
      render.fk_pump(ch)
      const got = read ? render.fk_probe_read(ch, BLOCK) : 0
      const out = read ? new Float32Array(memory.buffer, render.fk_probe_out(ch), BLOCK * 2).slice() : null
      clock += (BLOCK * 1000) / RATE
      prepare.fk_flush(ch)
      disk.tick()
      await Promise.all(disk.inFlight)
      return { got, out }
    }
    return { memory, prepare, render, ch, block, disk }
  }

  const standard = () =>
    new Map([
      [`blobs/${LONG}`, longWav()],
      [`blobs/${SHORT}`, new TextEncoder().encode('not a wav')],
      [`.faulk/cache/pcm/${SHORT}.pcm`, shortCache()],
    ])

  it('plays a clip start from memory then the ring, a jump on time and a short loop, without allocating', async () => {
    const r = rig(standard())
    for (let i = 0; i < 200; i++) await r.block(false)
    const allocs = [r.prepare.fk_allocs(ROLE_RENDER), r.prepare.fk_frees(ROLE_RENDER)]
    const play = async (
      source: number,
      start: number,
      end: number,
      loop: [number, number] | null,
      blocks: number,
      expected: (i: number, c: number) => number,
    ) => {
      r.render.fk_pump(r.ch)
      expect(r.render.fk_probe_start(r.ch, 0, 0, source, start, end, loop?.[0] ?? 0, loop?.[1] ?? 0)).toBe(1)
      let waited = 0
      let started = false
      for (let b = 0; b < blocks; b++) {
        const { got, out } = await r.block(true)
        for (let f = 0; f < BLOCK; f++) {
          const i = b * BLOCK + f
          const silent = out![2 * f] === 0 && out![2 * f + 1] === 0
          if (!started && silent && f >= got) {
            waited++
            continue
          }
          started = true
          expect([out![2 * f], out![2 * f + 1]], `frame ${i}`).toEqual([expected(i, 0), expected(i, 1)])
        }
      }
      return waited
    }
    // A clip start: resident, no wait, handed over to the ring after 1.5 s.
    expect(await play(0, CLIP_START, CLIP_START + 30 * RATE, null, 250, (i, c) => longValue(CLIP_START + i, c))).toBe(0)
    // A jump: a short wait, then on time.
    const jump = await play(0, 200_000, LONG_FRAMES, null, 100, (i, c) => longValue(200_000 + i, c))
    expect(jump).toBeGreaterThan(0)
    expect(jump).toBeLessThan(RATE / 4)
    // A short loop of the short source, from memory.
    const at = (i: number) => (i < 1_000 ? i : 1_000 + ((i - 1_000) % 800))
    expect(await play(1, 0, SHORT_FRAMES, [1_000, 1_800], 100, (i, c) => shortValue(at(i), c))).toBe(0)
    // A short loop of the long source, through the ring: a wait, then every wrap on time.
    const wrap = (i: number) => (i < 1_000 ? 299_000 + i : 300_000 + ((i - 1_000) % 1_600))
    expect(await play(0, 299_000, LONG_FRAMES, [300_000, 301_600], 150, (i, c) => longValue(wrap(i), c))).toBeGreaterThan(0)
    // A render writes the report (about 30 per second): enough frames for one.
    for (let i = 0; i < 4; i++) r.render.fk_render(r.ch, BLOCK)
    expect([r.prepare.fk_allocs(ROLE_RENDER), r.prepare.fk_frees(ROLE_RENDER)]).toEqual(allocs)
    const report = readFeedback(r.memory.buffer, r.prepare.fk_feedback(r.ch), r.prepare.fk_feedback_len(r.ch))!
    expect(report.diskUnderruns).toBe(0)
    expect(report.offlineSources).toBe(0)
    expect(report.pcmKib).toBeGreaterThan(0)
  }, 60_000)

  it('reports a source whose blob is missing as offline, until the blob comes back', async () => {
    const files = standard()
    const wav = files.get(`blobs/${LONG}`)!
    files.delete(`blobs/${LONG}`)
    const r = rig(files)
    const offline = async () => {
      // A render writes the report (about 30 per second): enough frames for one.
      for (let i = 0; i < 4; i++) r.render.fk_render(r.ch, BLOCK)
      return readFeedback(r.memory.buffer, r.prepare.fk_feedback(r.ch), r.prepare.fk_feedback_len(r.ch))!.offlineSources
    }
    for (let i = 0; i < 20; i++) await r.block(false)
    expect(await offline()).toBe(1)
    files.set(`blobs/${LONG}`, wav)
    for (let i = 0; i < 200; i++) await r.block(false)
    expect(await offline()).toBe(0)
  }, 60_000)
})
