import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'

// CT-3 (#39): the two ways of § 2.3 for the project → engine channel in the browser, measured
// on the render thread of a real AudioWorklet in Chromium, with the real synth rendering a note
// throughout. The prototype lives in src/lib/engine/proto/ (a crate built twice by
// scripts/build-proto.mjs, a clock worker, a prepare worker, a worklet). The numbers are printed
// for the pull request; the assertions hold the budgets of § 2.3 with a wide margin, because a
// CI machine is slower and noisier than a workstation.
//
// Way (a): plain memory, the page posts bounded bytes to the worklet, which copies them into a
// preallocated inbox; a growth is prepared in the worklet between two quanta.
// Way (b): shared memory (wasm32-wasip1-threads on the pinned stable), a Worker prepares and
// pushes on the rt ring inside the wasm memory, the worklet pops at the start of each block.

const proto = fileURLToPath(new URL('../../src/lib/engine/proto/', import.meta.url))
const build = `${proto}build/`

test.describe.configure({ mode: 'serial' })

test.beforeAll(() => {
  if (!existsSync(`${build}plain.wasm`) || !existsSync(`${build}shared.wasm`) || process.env.CI) {
    execFileSync(process.execPath, [fileURLToPath(new URL('../../scripts/build-proto.mjs', import.meta.url))], {
      stdio: 'inherit',
    })
  }
})

/** Serves the prototype's files from the page's origin, with the headers the server really sends. */
async function serveProto(page: Page, baseURL: string) {
  await page.route('**/proto/*', async (route) => {
    const name = new URL(route.request().url()).pathname.split('/').pop()!
    const response = await route.fetch({ url: new URL('/favicon.svg', baseURL).href })
    const wasm = name.endsWith('.wasm')
    await route.fulfill({
      response,
      contentType: wasm ? 'application/wasm' : 'text/javascript',
      body: readFileSync(wasm ? build + name : proto + name),
    })
  })
}

type Report = {
  blocks: number
  lateBlocks: number
  gapMaxUs: number
  gapMeanUs: number
  pumpCalls: number
  pumped: number
  pumpUsPerCall: number
  pumpUsPerMsg: number
  pumpMaxUs: number
  msgCount: number
  msgUsHandler: number
  bursts: { n: number; usPerMsg: number }[]
  grows: { us: number; before: number; after: number; notes: number; tracks: number; pumped: number; collected: number; detached: boolean }[]
  viewsRefreshed: number
  peak: number
  silentBlocks: number
  renderAllocs: number
  renderFrees: number
  applied: number
  notes: number
  tracksCapacity: number
  memoryBytes: number
  sampleRate: number
}

type Run = {
  mode: 'a' | 'b'
  ticksPerUs: number
  ticksPerUsCheck: number
  idle: Report
  load: Report
  worker?: {
    bursts: { ms: number; busy: number }[]
    grows: { ms: number; before: number; after: number; notes: number; tracks: number }[]
    counters: {
      prepare: { allocs: number; frees: number }
      render: { allocs: number; frees: number }
      busy: number
      applied: number
      notes: number
      tracksLen: number
      tracksCapacity: number
      role: number
    }
    maximumPages: number
  }
  workletRole: number
}

/** The whole measurement of one way, run inside the page. */
async function measure(page: Page, mode: 'a' | 'b'): Promise<Run> {
  // Worklet and worker scripts are loaded from blob URLs: a request made from a worklet
  // scope does not go through page.route, and a blob of an isolated page is isolated too.
  const sources = {
    clock: readFileSync(`${proto}clock-worker.js`, 'utf8'),
    worker: readFileSync(`${proto}prepare-worker.js`, 'utf8'),
    worklet: readFileSync(`${proto}proto-worklet.js`, 'utf8'),
  }
  return page.evaluate(async ({ mode, sources }) => {
    const blob = (source: string) => URL.createObjectURL(new Blob([source], { type: 'text/javascript' }))
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
    const clockBuffer = new SharedArrayBuffer(8)
    const clock = new BigInt64Array(clockBuffer)
    const clockWorker = new Worker(blob(sources.clock))
    clockWorker.postMessage(clockBuffer)
    const calibrate = async () => {
      const c0 = Atomics.load(clock, 0)
      const t0 = performance.now()
      await sleep(300)
      const c1 = Atomics.load(clock, 0)
      const t1 = performance.now()
      return Number(c1 - c0) / ((t1 - t0) * 1000)
    }
    await sleep(100)
    const ticksPerUs = await calibrate()

    const context = new AudioContext()
    await context.resume()
    await context.audioWorklet.addModule(blob(sources.worklet))
    const node = new AudioWorkletNode(context, 'faulk-proto', { numberOfInputs: 0, outputChannelCount: [2] })
    const fromWorklet: ((m: Record<string, unknown>) => void)[] = []
    node.port.onmessage = (e) => {
      const m = e.data
      if (m.type === 'error') throw new Error(`worklet: ${m.message}`)
      fromWorklet.shift()?.(m)
    }
    const ask = (message: Record<string, unknown>, transfer: Transferable[] = []) =>
      new Promise<Record<string, unknown>>((resolve) => {
        fromWorklet.push(resolve)
        node.port.postMessage(message, transfer)
      })

    let worker: Worker | undefined
    let askWorker: ((m: Record<string, unknown>) => Promise<Record<string, unknown>>) | undefined
    let memory: WebAssembly.Memory | undefined
    let maximumPages = 0
    let ready: Record<string, unknown>
    if (mode === 'a') {
      const bytes = await (await fetch('/proto/plain.wasm')).arrayBuffer()
      ready = await ask({ type: 'init', mode, bytes, clock: clockBuffer, ticksPerUs }, [bytes])
    } else {
      const bytes = await (await fetch('/proto/shared.wasm')).arrayBuffer()
      // 16 MiB initial (WASM_INITIAL_PAGES), the wasm32 cap as maximum when the browser allows it.
      for (const maximum of [65536, 32768, 16384]) {
        try {
          memory = new WebAssembly.Memory({ initial: 256, maximum, shared: true })
          maximumPages = maximum
          break
        } catch {
          /* try a smaller maximum */
        }
      }
      worker = new Worker(blob(sources.worker))
      const fromWorker: ((m: Record<string, unknown>) => void)[] = []
      worker.onmessage = (e) => {
        const m = e.data
        if (m.type === 'error') throw new Error(`prepare worker: ${m.message}`)
        fromWorker.shift()?.(m)
      }
      askWorker = (message) =>
        new Promise((resolve) => {
          fromWorker.push(resolve)
          worker!.postMessage(message)
        })
      const w = await askWorker({ type: 'init', bytes, memory, sampleRate: context.sampleRate })
      ready = await ask({ type: 'init', mode, bytes, memory, proto: w.proto, inst: w.inst, stackTop: w.stackTop, clock: clockBuffer, ticksPerUs })
    }
    node.connect(context.destination)

    // Idle: one second of rendering with no message, the baseline of the block gaps.
    await sleep(300)
    await ask({ type: 'reset' })
    await sleep(1000)
    const idle = (await ask({ type: 'report' })) as unknown as Report

    // Load: ten bursts of 100 messages, then three growths of a million-note snapshot plus the
    // track table at its limit (4096), each applied while the note keeps playing.
    await ask({ type: 'reset' })
    const workerBursts: Record<string, unknown>[] = []
    const workerGrows: Record<string, unknown>[] = []
    for (let burst = 0; burst < 10; burst++) {
      if (mode === 'a') {
        for (let i = 0; i < 100; i++) {
          const buf = new ArrayBuffer(16)
          const view = new DataView(buf)
          view.setUint32(0, 0, true) // param 0 (gain) ...
          view.setFloat32(4, 0.5 + i / 1000, true) // ... barely moved
          node.port.postMessage({ type: 'msg', burst, buf }, [buf])
        }
      } else {
        workerBursts.push(await askWorker!({ type: 'burst', n: 100, param: 0 }))
      }
      await sleep(50)
    }
    for (let i = 0; i < 3; i++) {
      if (mode === 'a') node.port.postMessage({ type: 'grow', notes: 1_000_000, tracks: 4096 })
      else workerGrows.push(await askWorker!({ type: 'grow', notes: 1_000_000, tracks: 4096 }))
      await sleep(200)
    }
    if (mode === 'b') await askWorker!({ type: 'collect' })
    await sleep(300)
    const load = (await ask({ type: 'report' })) as unknown as Report
    const counters = mode === 'b' ? await askWorker!({ type: 'counters' }) : undefined
    const ticksPerUsCheck = await calibrate()

    clockWorker.terminate()
    worker?.terminate()
    await context.close()
    return {
      mode,
      ticksPerUs,
      ticksPerUsCheck,
      idle,
      load,
      worker:
        mode === 'b'
          ? ({ bursts: workerBursts, grows: workerGrows, counters, maximumPages } as unknown as Run['worker'])
          : undefined,
      workletRole: ready.role as number,
    }
  }, { mode, sources })
}

function table(run: Run) {
  const { idle, load } = run
  const us = (n: number) => n.toFixed(2)
  const lines = [
    `way (${run.mode}) at ${load.sampleRate} Hz, clock ${run.ticksPerUs.toFixed(1)} ticks/µs (check ${run.ticksPerUsCheck.toFixed(1)})`,
    `  idle: ${idle.blocks} blocks, gap mean ${us(idle.gapMeanUs)} µs, max ${us(idle.gapMaxUs)} µs, late ${idle.lateBlocks}`,
    `  load: ${load.blocks} blocks, gap mean ${us(load.gapMeanUs)} µs, max ${us(load.gapMaxUs)} µs, late ${load.lateBlocks}, silent ${load.silentBlocks}, peak ${load.peak.toFixed(3)}`,
  ]
  if (run.mode === 'a') {
    const bursts = load.bursts.map((b) => b.usPerMsg)
    lines.push(
      `  per message on the render thread: handler ${us(load.msgUsHandler)} µs (${load.msgCount} messages); burst of 100 incl. event dispatch: ${us(Math.min(...bursts))}–${us(Math.max(...bursts))} µs/msg`,
    )
    for (const g of load.grows)
      lines.push(
        `  growth on the render thread: ${us(g.us)} µs (prepare 1 M notes + 4096 tracks, pump ${g.pumped}, collect ${g.collected}), memory ${g.before >> 20} → ${g.after >> 20} MiB, buffer detached ${g.detached}`,
      )
    lines.push(`  render-side allocations ${load.renderAllocs}, frees ${load.renderFrees} (prepare runs there in this way)`)
  } else {
    lines.push(
      `  per message on the render thread: pump ${us(load.pumpUsPerMsg)} µs/msg (${load.pumped} messages in ${load.pumpCalls} pumps, ${us(load.pumpUsPerCall)} µs per pump incl. empty ones, longest pump ${us(load.pumpMaxUs)} µs incl. the swaps; idle pump ${us(idle.pumpUsPerCall)} µs)`,
    )
    for (const [i, g] of run.worker!.grows.entries())
      lines.push(
        `  growth: prepare in the worker ${g.ms.toFixed(2)} ms (1 M notes + 4096 tracks, codes ${g.notes}/${g.tracks}), memory ${g.before >> 20} → ${g.after >> 20} MiB; worker burst ${i < run.worker!.bursts.length ? run.worker!.bursts[i].ms.toFixed(3) : '-'} ms/100`,
      )
    const c = run.worker!.counters
    lines.push(
      `  render-side allocations ${c.render.allocs}, frees ${c.render.frees}; prepare-side ${c.prepare.allocs}/${c.prepare.frees}; ring busy ${c.busy}; maximum ${run.worker!.maximumPages} pages; roles worker ${c.role} / worklet ${run.workletRole}`,
    )
  }
  lines.push(`  views re-taken ${load.viewsRefreshed} times, memory ${load.memoryBytes >> 20} MiB, notes table ${load.notes}, track capacity ${load.tracksCapacity}`)
  return lines.join('\n')
}

test.describe('CT-3: the project → engine channel in the browser', () => {
  test('way (a), plain memory: messages and growth on the render thread', async ({ page, baseURL }) => {
    await serveProto(page, baseURL!)
    await page.goto('/app')
    await page.getByRole('button', { name: 'Start audio' }).first().click()
    const run = await measure(page, 'a')
    console.log(table(run))
    const { load } = run
    expect(run.ticksPerUs).toBeGreaterThan(1)
    expect(load.msgCount).toBe(1000)
    expect(load.peak).toBeGreaterThan(0.01)
    expect(load.grows).toHaveLength(3)
    for (const g of load.grows) {
      expect(g.notes).toBe(0)
      expect(g.tracks).toBe(0)
      expect(g.pumped).toBe(2)
    }
    // Attack: memory.grow while the worklet holds views. A plain memory detaches the old
    // buffer; the views are re-taken at the next block and the audio goes on.
    expect(load.grows[0].detached).toBe(true)
    expect(load.viewsRefreshed).toBeGreaterThanOrEqual(1)
    expect(load.notes).toBe(1_000_000)
    expect(load.tracksCapacity).toBe(4096)
    // The budget, with margin: a message costs under 50 µs of render thread, dispatch included.
    for (const b of load.bursts) expect(b.usPerMsg).toBeLessThan(50)
    // Prepare on the render thread is the price of this way: it allocates and frees there.
    expect(load.renderAllocs).toBeGreaterThan(0)
  })

  test('way (b), shared memory: a Worker prepares, the worklet pumps', async ({ page, baseURL }) => {
    await serveProto(page, baseURL!)
    await page.goto('/app')
    await page.getByRole('button', { name: 'Start audio' }).first().click()
    const run = await measure(page, 'b')
    console.log(table(run))
    const { load } = run
    const c = run.worker!.counters
    expect(run.ticksPerUs).toBeGreaterThan(1)
    // Two instances on one memory: the worker is the prepare side, the worklet the render side,
    // each with its own thread-local role and its own stack.
    expect(c.role).toBe(1)
    expect(run.workletRole).toBe(2)
    expect(load.pumped).toBe(1000 + 6)
    expect(c.applied).toBe(1000 + 6)
    expect(c.busy).toBe(0)
    expect(load.peak).toBeGreaterThan(0.01)
    // The render side never allocated nor freed: the growths were prepared in the worker and
    // the old tables dropped there.
    expect(load.renderAllocs).toBe(0)
    expect(load.renderFrees).toBe(0)
    expect(c.prepare.frees).toBeGreaterThan(0)
    expect(c.notes).toBe(1_000_000)
    expect(c.tracksCapacity).toBe(4096)
    for (const g of run.worker!.grows) {
      expect(g.notes).toBe(0)
      expect(g.tracks).toBe(0)
      expect(g.after).toBeGreaterThanOrEqual(g.before)
    }
    expect(run.worker!.grows[0].after).toBeGreaterThan(run.worker!.grows[0].before)
    // Attack: memory.grow while the worklet holds views. A shared memory keeps the old buffer
    // valid; the worklet re-took its views (memory.buffer changed) and the audio went on.
    expect(load.viewsRefreshed).toBeGreaterThanOrEqual(1)
    expect(load.silentBlocks).toBe(0)
    // The budget, with margin: a pump costs under 20 µs per message.
    expect(load.pumpUsPerMsg).toBeLessThan(20)
    expect(run.worker!.maximumPages).toBeGreaterThanOrEqual(16384)
  })

  test('attack: memory.grow refused and views across a growth, both memories', async ({ page, baseURL }) => {
    await serveProto(page, baseURL!)
    await page.goto('/app')
    const result = await page.evaluate(async () => {
      const wasi = () => ({
        environ_get: () => 0,
        environ_sizes_get: () => 0,
        fd_write: () => 0,
        proc_exit: () => {
          throw new Error('exit')
        },
        sched_yield: () => 0,
      })
      const shared = new WebAssembly.Module(await (await fetch('/proto/shared.wasm')).arrayBuffer())
      const plain = new WebAssembly.Module(await (await fetch('/proto/plain.wasm')).arrayBuffer())

      // A memory that cannot grow past 20 MiB stands in for the 4 GiB cap: the same
      // `memory.grow` refusal, reached by the same fallible allocation.
      const small = new WebAssembly.Memory({ initial: 256, maximum: 320, shared: true })
      const capped = new WebAssembly.Instance(shared, { env: { memory: small }, wasi_snapshot_preview1: wasi() }).exports as Record<string, CallableFunction>
      const proto = capped.fk_proto_new(1)
      const refused = capped.fk_proto_prepare_notes(proto, 10_000_000)
      const aliveAfterRefusal = capped.fk_proto_push_param(proto, 0, 1)
      const smallerOk = capped.fk_proto_prepare_notes(proto, 1000)

      // Views across a growth: shared memory keeps the old buffer, plain memory detaches it.
      const grown = new WebAssembly.Memory({ initial: 256, maximum: 1024, shared: true })
      const oldShared = new Uint8Array(grown.buffer)
      oldShared[0] = 7
      grown.grow(16)
      const sharedKept = { sameObject: grown.buffer === oldShared.buffer, oldLength: oldShared.byteLength, oldReadable: oldShared[0] }
      const plainInstance = new WebAssembly.Instance(plain, {}).exports as { memory: WebAssembly.Memory }
      const oldPlain = new Uint8Array(plainInstance.memory.buffer)
      const plainLengthBefore = oldPlain.byteLength
      plainInstance.memory.grow(16)
      const plainDetached = { lengthBefore: plainLengthBefore, lengthAfter: oldPlain.byteLength }
      return { refused, aliveAfterRefusal, smallerOk, sharedKept, plainDetached }
    })
    expect(result.refused).toBe(1) // MsgError::OutOfMemory, not a trap
    expect(result.aliveAfterRefusal).toBe(0)
    expect(result.smallerOk).toBe(0)
    expect(result.sharedKept.sameObject).toBe(false)
    expect(result.sharedKept.oldLength).toBe(256 << 16)
    expect(result.sharedKept.oldReadable).toBe(7)
    expect(result.plainDetached.lengthBefore).toBeGreaterThan(0)
    expect(result.plainDetached.lengthAfter).toBe(0)
  })
})
