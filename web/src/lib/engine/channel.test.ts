// The shipped engine module (engine.wasm, wasm32-wasip1-threads, built by `pnpm test`) under
// Node: two instances on one shared memory, as the prepare Worker and the AudioWorklet run it
// (MO-2). The streams are the Rust recordings of crates/engine-wasm/tests/fixtures: nothing is
// encoded here.
import { readFileSync } from 'node:fs'
import { Worker } from 'node:worker_threads'
import { describe, expect, it, vi } from 'vitest'
import { becomeRenderInstance, createChannel, instantiate, MAX_FRAMES, ROLE_PREPARE, ROLE_RENDER, wasiImports, warmUp, type EngineExports } from './engine-abi'
import { peakOf, readFeedback } from './feedback'
import { PushQueue } from './push-queue'

const bytes = new Uint8Array(readFileSync(new URL('./wasm/engine.wasm', import.meta.url))).buffer
const module = new WebAssembly.Module(bytes)
const fixtures = new URL('../../../../crates/engine-wasm/tests/fixtures/', import.meta.url)
/** libm differs slightly between the native targets and wasm32 (as in the Rust parity tests). */
const TOLERANCE = 1e-5
const QUANTUM = 128

type Processor = {
  port: { onmessage: ((event: MessageEvent) => void) | null; sent: unknown[] }
  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean
}

/** The processor worklet.ts registers, built under a stub of the AudioWorkletGlobalScope. */
let registered: (new () => Processor) | null = null

async function shippedProcessor(): Promise<Processor> {
  const scope = globalThis as unknown as Record<string, unknown>
  scope.sampleRate = 48_000
  // The context frame of the block (#499): the stub does not advance it, nothing here is stamped.
  scope.currentFrame = 0
  scope.AudioWorkletProcessor = class {
    port = {
      onmessage: null,
      sent: [] as unknown[],
      postMessage(message: unknown) {
        this.sent.push(message)
      },
    }
  }
  scope.registerProcessor = (_name: string, processor: new () => Processor) => {
    registered = processor
  }
  // The module registers once (imports are cached): the class is kept for the next call.
  await import('./worklet')
  return new registered!()
}

type Step = { push: Uint8Array } | { render: number }

/** The fixture format of crates/engine-wasm/tests/parity.rs. */
function steps(name: string): Step[] {
  const bytes = new Uint8Array(readFileSync(new URL(name, fixtures)))
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const out: Step[] = []
  for (let at = 0; at + 5 <= bytes.length;) {
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

/** An instance of the module whose WASI functions count their calls, by name, in `calls`. */
function countingInstance(memory: WebAssembly.Memory, calls: Record<string, number>): EngineExports {
  const wasi = wasiImports(memory) as unknown as Record<string, (...args: never[]) => unknown>
  const counted = Object.fromEntries(
    Object.entries(wasi).map(([name, f]) => [
      name,
      (...args: never[]) => {
        calls[name] = (calls[name] ?? 0) + 1
        return f(...args)
      },
    ]),
  )
  return new WebAssembly.Instance(module, { env: { memory }, wasi_snapshot_preview1: counted }).exports as unknown as EngineExports
}

describe('the WASI functions the engine module imports', () => {
  it('are exactly the ones the page provides', () => {
    const imported = WebAssembly.Module.imports(module)
      .filter((i) => i.module === 'wasi_snapshot_preview1')
      .map((i) => i.name)
      .sort()
    expect(imported).toEqual(Object.keys(wasiImports(sharedMemory())).sort())
  })

  it('never read the clock (#846): the render calls none of them, and nothing calls clock_time_get', () => {
    const memory = sharedMemory()
    const prepareCalls: Record<string, number> = {}
    const renderCalls: Record<string, number> = {}
    const prepare = countingInstance(memory, prepareCalls)
    const setup = createChannel(prepare, 48_000)
    const render = countingInstance(memory, renderCalls)
    becomeRenderInstance(render, setup)
    const ch = setup.channel
    let warmed = false
    let blocks = 0
    for (const step of steps('session.stream')) {
      if ('push' in step) expect(push(prepare, memory, ch, step.push)).toBe(0)
      else {
        if (!warmed && blocks > 0) {
          // The warm-up's private channel (#355) may seed its hash maps: the one call allowed.
          warmed = true
          const before = { ...renderCalls }
          expect(warmUp(render, 48_000)).toBe(0)
          for (const name of Object.keys(renderCalls)) if (name !== 'random_get') expect(renderCalls[name]).toBe(before[name])
          for (const name of Object.keys(renderCalls)) delete renderCalls[name]
        }
        for (let b = 0; b < step.render; b++, blocks++) render.fk_render(ch, QUANTUM)
      }
    }
    expect(warmed).toBe(true)
    expect(blocks).toBeGreaterThan(100)
    expect(renderCalls).toEqual({}) // the audio path: no WASI call at all
    expect(prepareCalls.clock_time_get).toBeUndefined()
    // Were it called, it would read no clock and name itself: here, in the growth test's two
    // threads (contended locks) and in every other test of the module.
    expect(() => wasiImports(memory).clock_time_get()).toThrow(/clock_time_get/)
  })
})

describe('the shipped engine module, two instances on one shared memory', () => {
  it('keeps a stack, a thread-local block and a role per instance', () => {
    const memory = sharedMemory()
    const { prepare, render } = channel(memory)
    expect(prepare.fk_role()).toBe(ROLE_PREPARE)
    expect(render.fk_role()).toBe(ROLE_RENDER)
    expect(Number(render.__stack_pointer.value)).not.toBe(Number(prepare.__stack_pointer.value))
  })

  it.each([false, true])('renders the recorded session as the native engine does, without allocating (warmed up: %s)', (warmed) => {
    const memory = sharedMemory()
    const { prepare, render, ch } = channel(memory)
    const native = new Float32Array(readFileSync(new URL('session.native.f32', fixtures)).buffer.slice(0))
    const out: number[] = []
    const allocs = [render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]
    let warmDone = !warmed
    let rendered = 0
    for (const step of steps('session.stream')) {
      if ('push' in step) expect(push(prepare, memory, ch, step.push)).toBe(0)
      else {
        // After the first render: the pump has applied the setup, so the real channel holds its track.
        if (!warmDone && rendered > 0) {
          // The warm-up of the apply path (#355) in the middle of the session, notes about to
          // sound: it runs on a private channel, so the populated real one is untouched.
          warmDone = true
          const before = [0, 1, 2, 3, 4].map((which) => render.fk_host_counter(ch, which))
          const roleAllocs = [render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]
          expect(warmUp(render, 48_000)).toBe(0)
          expect([0, 1, 2, 3, 4].map((which) => render.fk_host_counter(ch, which))).toEqual(before)
          expect(before[4]).toBeGreaterThan(0) // the real channel holds a track already
          expect(render.fk_role()).toBe(ROLE_RENDER) // the role is back
          // Its allocations are not the render role's: the audio path's counters did not move.
          expect([render.fk_allocs(ROLE_RENDER), render.fk_frees(ROLE_RENDER)]).toEqual(roleAllocs)
          expect(render.fk_frees(0)).toBe(render.fk_allocs(0)) // everything the warm-up took was freed
        }
        rendered++
        for (let b = 0; b < step.render; b++) {
          const n = render.fk_render(ch, QUANTUM)
          const left = new Float32Array(memory.buffer, render.fk_left(ch), MAX_FRAMES)
          const right = new Float32Array(memory.buffer, render.fk_right(ch), MAX_FRAMES)
          for (let i = 0; i < n; i++) out.push(left[i])
          for (let i = 0; i < n; i++) out.push(right[i])
        }
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

  it('warms up in a scope without `crypto`, as the AudioWorkletGlobalScope is', async () => {
    const memory = sharedMemory()
    const prepare = instantiate(module, memory)
    const setup = createChannel(prepare, 48_000)
    const processor = await shippedProcessor()
    vi.stubGlobal('crypto', undefined)
    try {
      processor.port.onmessage!({ data: { type: 'init', bytes: bytes.slice(0), memory, setup } } as MessageEvent)
    } finally {
      vi.unstubAllGlobals()
    }
    expect(processor.port.sent).toEqual([{ type: 'ready' }]) // no `warm` field: the warm-up went through
  })

  it('puts the render instance back when the warm-up traps', () => {
    const memory = sharedMemory()
    const { render } = channel(memory)
    const stack = render.__stack_pointer.value
    const trapping = {
      ...render,
      fk_warm: () => {
        render.fk_set_role(0)
        render.__stack_pointer.value = Number(stack) - 4096
        throw new WebAssembly.RuntimeError('unreachable')
      },
    }
    expect(warmUp(trapping, 48_000)).toBe(-1)
    expect(render.__stack_pointer.value).toBe(stack)
    expect(render.fk_role()).toBe(ROLE_RENDER)
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
    expect(report!.tracks).toHaveLength(1)
    expect(peakOf(report!.tracks[0])).toBeGreaterThan(0.05)
    expect(peakOf(report!.master)).toBeGreaterThan(0.05)
    expect(report!.cpuLoad).toBeNull() // no block was timed (`fk_add_render_time`): not measured
    expect(report!.cpuPeak).toBeNull()
    expect(report!.transport).toBe('stopped')
  })

  it('reports the CPU load the shipped worklet times on each block (#486)', async () => {
    const memory = sharedMemory()
    const prepare = instantiate(module, memory)
    const setup = createChannel(prepare, 48_000)
    const ch = setup.channel
    const processor = await shippedProcessor()
    processor.port.onmessage!({ data: { type: 'init', bytes: bytes.slice(0), memory, setup } } as MessageEvent)
    expect(processor.port.sent).toEqual([{ type: 'ready' }])
    for (const step of steps('session.stream')) {
      if (!('push' in step)) break
      expect(push(prepare, memory, ch, step.push)).toBe(0)
    }
    const left = new Float32Array(QUANTUM)
    const right = new Float32Array(QUANTUM)
    for (let b = 0; b < 40; b++) processor.process([], [[left, right]])
    const report = readFeedback(memory.buffer, prepare.fk_feedback(ch), prepare.fk_feedback_len(ch))
    expect(report!.voices).toBe(1)
    // Node has `performance.now()`: a fine clock, the render of a voice takes some time.
    expect(report!.cpuLoad).toBeGreaterThan(0)
    expect(report!.cpuLoad).toBeLessThanOrEqual(1)
    expect(report!.cpuPeak).toBeGreaterThanOrEqual(report!.cpuLoad!)
    expect(report!.cpuPeak).toBeLessThanOrEqual(1)
  })

  it('grows the shared memory from the prepare thread while the shipped worklet plays', async () => {
    const memory = sharedMemory()
    const prepare = instantiate(module, memory)
    const setup = createChannel(prepare, 48_000)
    const ch = setup.channel
    // The shipped processor (worklet.ts), its AudioWorkletGlobalScope stubbed.
    const processor = await shippedProcessor()
    processor.port.onmessage!({ data: { type: 'init', bytes: bytes.slice(0), memory, setup } } as MessageEvent)
    expect(processor.port.sent).toEqual([{ type: 'ready' }])
    for (const step of steps('session.stream')) {
      if (!('push' in step)) break
      expect(push(prepare, memory, ch, step.push)).toBe(0)
    }
    const growth = steps('growth.stream').flatMap((s) => ('push' in s ? [s.push] : []))
    const done = new Int32Array(new SharedArrayBuffer(8))
    // The prepare side on another thread: its own instance of the module, as the Worker,
    // with its own stack and thread-local block, as every instance after the first needs.
    const stack = prepare.fk_alloc(1 << 20, 16)
    const tls = prepare.fk_alloc(Number(prepare.__tls_size.value), Number(prepare.__tls_align.value))
    const thread = { stackTop: stack + (1 << 20), tls }
    const worker = new Worker(PREPARE_THREAD, { eval: true, workerData: { module, memory, ch, growth, done, thread } })
    const errors: unknown[] = []
    worker.on('error', (e) => errors.push(e))
    const allocs = [prepare.fk_allocs(ROLE_RENDER), prepare.fk_frees(ROLE_RENDER)]
    const before = memory.buffer.byteLength
    const left = new Float32Array(QUANTUM)
    const right = new Float32Array(QUANTUM)
    let blocks = 0
    let silent = 0
    const held = () => prepare.fk_host_counter(ch, 4) // read only, between two blocks
    const deadline = Date.now() + 60_000
    while ((Atomics.load(done, 0) === 0 || held() < 4096) && Date.now() < deadline) {
      processor.process([], [[left, right]])
      let peak = 0
      for (let i = 0; i < QUANTUM; i++) peak = Math.max(peak, Math.abs(left[i]))
      if (peak === 0) silent++
      blocks++
      if (Atomics.load(done, 0) !== 0) prepare.fk_flush(ch) // the thread is done: nothing else flushes
    }
    await worker.terminate()
    expect(errors).toEqual([])
    expect(Atomics.load(done, 1)).toBe(0) // the prepare thread saw no error code
    expect(held()).toBe(4096)
    expect(prepare.fk_host_counter(ch, 3)).toBe(4096) // track table capacity
    expect(prepare.fk_host_counter(ch, 1)).toBe(0) // skipped
    expect(memory.buffer.byteLength).toBeGreaterThan(before)
    // The processor re-took its views on the grown memory.
    expect((processor as unknown as { buffer: ArrayBufferLike }).buffer).toBe(memory.buffer)
    expect(silent).toBe(0)
    expect(blocks).toBeGreaterThan(64)
    expect([prepare.fk_allocs(ROLE_RENDER), prepare.fk_frees(ROLE_RENDER)]).toEqual(allocs)
    expect(processor.port.sent).toHaveLength(1) // nothing posted during audio
  }, 90_000)

  it('resends what a busy engine refused, in order, and loses nothing', () => {
    const memory = sharedMemory()
    const { prepare, render, ch } = channel(memory)
    const stopped: number[] = []
    const queue = new PushQueue(prepare, memory, ch, (code) => stopped.push(code))
    for (const step of steps('session.stream')) {
      if (!('push' in step)) break
      queue.push(step.push)
    }
    // Without rendering: 4 + 8 190 rt messages for a ring and a queue of 4 096 each.
    for (const step of steps('growth.stream')) if ('push' in step) queue.push(step.push)
    expect(queue.waiting).toBeGreaterThan(0)
    for (let b = 0; b < 400 && (queue.waiting > 0 || render.fk_host_counter(ch, 4) < 4096); b++) {
      queue.flush()
      render.fk_render(ch, QUANTUM)
    }
    expect(stopped).toEqual([])
    expect(queue.waiting).toBe(0)
    expect(render.fk_host_counter(ch, 4)).toBe(4096)
    expect(prepare.fk_prepare_counter(ch, 0)).toBe(0) // rejected
  })

  it('turns a refused memory.grow into a typed error and keeps playing', () => {
    // A memory capped at 24 MiB (16 MiB initial) stands in for the 4 GiB cap.
    const memory = sharedMemory(384)
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
  clock_time_get() { throw new Error('clock_time_get called (#846)') }, environ_get: () => 0, environ_sizes_get: () => 0, fd_write: () => 0,
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
