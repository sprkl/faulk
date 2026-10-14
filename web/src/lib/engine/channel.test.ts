// The shipped engine module (engine.wasm, wasm32-wasip1-threads, built by `pnpm test`) under
// Node: two instances on one shared memory, as the prepare Worker and the AudioWorklet run it
// (MO-2). The streams are the Rust recordings of crates/engine-wasm/tests/fixtures: nothing is
// encoded here.
import { readFileSync } from 'node:fs'
import { Worker } from 'node:worker_threads'
import { describe, expect, it } from 'vitest'
import { becomeRenderInstance, createChannel, instantiate, MAX_FRAMES, ROLE_PREPARE, ROLE_RENDER, type EngineExports } from './engine-abi'
import { readFeedback } from './feedback'

const module = new WebAssembly.Module(readFileSync(new URL('./wasm/engine.wasm', import.meta.url)))
const fixtures = new URL('../../../../crates/engine-wasm/tests/fixtures/', import.meta.url)
/** libm differs slightly between the native targets and wasm32 (as in the Rust parity tests). */
const TOLERANCE = 1e-5
const QUANTUM = 128

type Step = { push: Uint8Array } | { render: number }

/** The fixture format of crates/engine-wasm/tests/parity.rs. */
function steps(name: string): Step[] {
  const bytes = new Uint8Array(readFileSync(new URL(name, fixtures)))
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const out: Step[] = []
  for (let at = 0; at + 5 <= bytes.length; ) {
    const n = view.getUint32(at + 1, true)
    if (bytes[at] === 0) {
      out.push({ push: bytes.slice(at + 5, at + 5 + n) })
      at += 5 + n
    } else {
      out.push({ render: n })
      at += 5
    }
  }
  return out
}

/** The two instances of a channel on `memory`. */
function channel(memory: WebAssembly.Memory) {
  const prepare = instantiate(module, memory)
  const setup = createChannel(prepare, 48_000)
  const render = instantiate(module, memory)
  becomeRenderInstance(render, setup)
  return { prepare, render, ch: setup.channel }
}

function push(e: EngineExports, memory: WebAssembly.Memory, ch: number, bytes: Uint8Array): number {
  const at = e.fk_inbox(ch, bytes.length)
  if (!at) return -8
  new Uint8Array(memory.buffer, at, bytes.length).set(bytes)
  return e.fk_push(ch, at, bytes.length)
}

const sharedMemory = (maximum = 65536) => new WebAssembly.Memory({ initial: 256, maximum, shared: true })

describe('the shipped engine module, two instances on one shared memory', () => {
  it('keeps a stack, a thread-local block and a role per instance', () => {
    const memory = sharedMemory()
    const { prepare, render } = channel(memory)
    expect(prepare.fk_role()).toBe(ROLE_PREPARE)
    expect(render.fk_role()).toBe(ROLE_RENDER)
    expect(Number(render.__stack_pointer.value)).not.toBe(Number(prepare.__stack_pointer.value))
  })

  it('renders the recorded session as the native engine does, without allocating', () => {
    const memory = sharedMemory()
    const { prepare, render, ch } = channel(memory)
    const native = new Float32Array(readFileSync(new URL('session.native.f32', fixtures)).buffer.slice(0))
    const out: number[] = []
    const allocs = [render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]
    for (const step of steps('session.stream')) {
      if ('push' in step) expect(push(prepare, memory, ch, step.push)).toBe(0)
      else
        for (let b = 0; b < step.render; b++) {
          const n = render.fk_render(ch, QUANTUM)
          const left = new Float32Array(memory.buffer, render.fk_left(ch), MAX_FRAMES)
          const right = new Float32Array(memory.buffer, render.fk_right(ch), MAX_FRAMES)
          for (let i = 0; i < n; i++) out.push(left[i])
          for (let i = 0; i < n; i++) out.push(right[i])
        }
    }
    expect(out.length).toBe(native.length)
    let worst = 0
    for (let i = 0; i < out.length; i++) worst = Math.max(worst, Math.abs(out[i] - native[i]))
    expect(worst).toBeLessThanOrEqual(TOLERANCE)
    expect(Math.max(...native.map(Math.abs))).toBeGreaterThan(0.1)
    expect([render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]).toEqual(allocs)
    expect(prepare.fk_prepare_counter(ch, 0)).toBe(0) // rejected
    expect(prepare.fk_allocs(ROLE_PREPARE)).toBeGreaterThan(0)
  })

  it('writes a report the page reads', () => {
    const memory = sharedMemory()
    const { prepare, render, ch } = channel(memory)
    for (const step of steps('session.stream')) {
      if (!('push' in step)) break
      push(prepare, memory, ch, step.push)
    }
    for (let b = 0; b < 40; b++) render.fk_render(ch, QUANTUM)
    const report = readFeedback(memory.buffer, prepare.fk_feedback(ch), prepare.fk_feedback_len(ch))
    expect(report).not.toBeNull()
    expect(report!.epoch).toBe(1)
    expect(report!.rejected).toBe(0)
    expect(report!.voices).toBe(1)
    expect(report!.peaks).toHaveLength(1)
    expect(report!.peaks[0]).toBeGreaterThan(0.05)
  })

  it('grows the shared memory from the prepare thread while the render instance plays', async () => {
    const memory = sharedMemory()
    const { prepare, render, ch } = channel(memory)
    for (const step of steps('session.stream')) {
      if (!('push' in step)) break
      expect(push(prepare, memory, ch, step.push)).toBe(0)
    }
    const growth = steps('growth.stream').flatMap((s) => ('push' in s ? [s.push] : []))
    const done = new Int32Array(new SharedArrayBuffer(8))
    // The prepare side on another thread: its own instance of the module, as the Worker.
    // Its own stack and thread-local block, as every instance after the first needs.
    const stack = prepare.fk_alloc(1 << 20, 16)
    const tls = prepare.fk_alloc(Number(prepare.__tls_size.value), Number(prepare.__tls_align.value))
    const thread = { stackTop: stack + (1 << 20), tls }
    const worker = new Worker(PREPARE_THREAD, { eval: true, workerData: { module, memory, ch, growth, done, thread } })
    const errors: unknown[] = []
    worker.on('error', (e) => errors.push(e))
    const allocs = [render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]
    const before = memory.buffer.byteLength
    let buffer = memory.buffer
    let views = 0
    let blocks = 0
    let silent = 0
    let left = new Float32Array(0)
    const deadline = Date.now() + 60_000
    while ((Atomics.load(done, 0) === 0 || render.fk_host_counter(ch, 4) < 4096) && Date.now() < deadline) {
      const n = render.fk_render(ch, QUANTUM)
      if (memory.buffer !== buffer || left.length === 0) {
        buffer = memory.buffer
        left = new Float32Array(buffer, render.fk_left(ch), MAX_FRAMES)
        views++
      }
      let peak = 0
      for (let i = 0; i < n; i++) peak = Math.max(peak, Math.abs(left[i]))
      if (peak === 0) silent++
      blocks++
      if (Atomics.load(done, 0) !== 0) prepare.fk_flush(ch) // nothing else runs here once the thread is done
    }
    await worker.terminate()
    expect(errors).toEqual([])
    expect(Atomics.load(done, 1)).toBe(0) // the prepare thread saw no error code
    expect(render.fk_host_counter(ch, 4)).toBe(4096) // tracks held
    expect(render.fk_host_counter(ch, 3)).toBe(4096) // track table capacity
    expect(render.fk_host_counter(ch, 1)).toBe(0) // skipped
    expect(memory.buffer.byteLength).toBeGreaterThan(before)
    expect(views).toBeGreaterThan(1)
    expect(silent).toBe(0)
    expect(blocks).toBeGreaterThan(64)
    expect([render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]).toEqual(allocs)
  }, 90_000)

  it('turns a refused memory.grow into a typed error and keeps playing', () => {
    // A memory capped at 20 MiB (16 MiB initial) stands in for the 4 GiB cap.
    const memory = sharedMemory(320)
    const { prepare, render, ch } = channel(memory)
    const session = steps('session.stream')
    for (const step of session) {
      if (!('push' in step)) break
      expect(push(prepare, memory, ch, step.push)).toBe(0)
    }
    const codes: number[] = []
    for (const step of steps('growth.stream')) {
      if ('push' in step) codes.push(push(prepare, memory, ch, step.push))
      else for (let b = 0; b < 64; b++) render.fk_render(ch, QUANTUM)
      if (codes.at(-1) === -8) break
    }
    expect(codes.at(-1)).toBe(-8) // MsgError::OutOfMemory
    expect(prepare.fk_stopped_at(ch)).toBeGreaterThanOrEqual(0)
    // The channel lives on: the note still sounds and a new message goes through.
    let peak = 0
    for (let b = 0; b < 8; b++) {
      const n = render.fk_render(ch, QUANTUM)
      const left = new Float32Array(memory.buffer, render.fk_left(ch), MAX_FRAMES)
      for (let i = 0; i < n; i++) peak = Math.max(peak, Math.abs(left[i]))
    }
    expect(peak).toBeGreaterThan(0.05)
    const param = session.filter((s) => 'push' in s)[1] as { push: Uint8Array }
    expect(push(prepare, memory, ch, param.push)).toBe(0)
  })
})

/**
 * The prepare thread: pushes the growth as fast as it can, waits and resends the rest of a
 * push when the engine is busy (-9), and flushes. `done[0]` = 1 at the end, `done[1]` = the
 * first other error code.
 */
const PREPARE_THREAD = `
const { workerData } = require('node:worker_threads')
const { module, memory, ch, growth, done, thread } = workerData
const wasi = {
  random_get(p, n) { const t = new Uint8Array(n); crypto.getRandomValues(t); new Uint8Array(memory.buffer, p, n).set(t); return 0 },
  environ_get: () => 0, environ_sizes_get: () => 0, fd_write: () => 0,
  proc_exit(c) { throw new Error('exit ' + c) }, sched_yield: () => 0,
}
const e = new WebAssembly.Instance(module, { env: { memory }, wasi_snapshot_preview1: wasi }).exports
e.__stack_pointer.value = thread.stackTop
e.__wasm_init_tls(thread.tls)
e.fk_set_role(1)
const sleep = new Int32Array(new SharedArrayBuffer(4))
for (let bytes of growth) {
  for (;;) {
    const at = e.fk_inbox(ch, bytes.length)
    new Uint8Array(memory.buffer, at, bytes.length).set(bytes)
    const code = e.fk_push(ch, at, bytes.length)
    if (code === 0) break
    if (code !== -9) { Atomics.store(done, 1, code); break }
    bytes = bytes.slice(e.fk_stopped_at(ch))
    Atomics.wait(sleep, 0, 0, 2)
    e.fk_flush(ch)
  }
}
Atomics.store(done, 0, 1)
`
