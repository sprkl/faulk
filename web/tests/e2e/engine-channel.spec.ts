import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { ceiling } from './support'

// CT-3 (#39): the two ways of § 2.3 for the project → engine channel in the browser, measured
// on the render thread of a real AudioWorklet in Chromium, with the real synth rendering a note
// throughout. The prototype lives in src/lib/engine/proto/ (a crate built twice by
// scripts/build-proto.mjs, a clock worker, a prepare worker, a worklet). The numbers are printed
// for the pull request; the assertions hold the budgets of § 2.3 through ceiling() (support.ts) in every mode, a single sample being too noisy for the strict
// budget (the shipped worklet holds it, engine-mo2.spec.ts), because a
// CI machine is slower and noisier than a workstation.
//
// Way (a): plain memory, the page posts bounded bytes to the worklet, which copies them into a
// preallocated inbox; a growth is prepared in the worklet between two quanta.
// Way (b): shared memory (wasm32-wasip1-threads on the pinned stable), a Worker prepares and
// pushes on the rt ring inside the wasm memory, the worklet pops at the start of each block.

const proto = fileURLToPath(new URL('../../src/lib/engine/proto/', import.meta.url))
const build = `${proto}build/`

test.describe.configure({ mode: 'serial' })

// One measurement is enough: the prototype does not depend on the server, so the suite runs
// under the dev project only (the preview project would repeat it on a busier machine).
test.skip(({ browserName }) => Boolean(browserName) && test.info().project.name !== 'dev', 'measured once, under the dev project')

/** Newest modification time under a directory (the crate's sources and manifest). */
function newest(dir: string): number {
  let t = 0
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'target') continue
    const path = `${dir}/${entry.name}`
    t = Math.max(t, entry.isDirectory() ? newest(path) : statSync(path).mtimeMs)
  }
  return t
}

test.beforeAll(() => {
  // Build the prototype when its artefacts are missing or older than the crate or the
  // engine crates it depends on; a fresh checkout in CI builds once.
  const outputs = [`${build}plain.wasm`, `${build}shared.wasm`]
  const crates = fileURLToPath(new URL('../../../crates/', import.meta.url))
  const sources = [`${proto}crate`, `${crates}engine`, `${crates}engine-wasm`]
  const stale =
    outputs.some((o) => !existsSync(o)) ||
    Math.min(...outputs.map((o) => statSync(o).mtimeMs)) < Math.max(...sources.map(newest))
  if (stale) {
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
  fills: { us: number; n: number; failed: number; pumped: number }[]
  skipped: number
  garbageRefused: number
  viewsRefreshed: number
  peak: number
  silentBlocks: number
  renderAllocs: number
  renderFrees: number
  applied: number
  notes: number
  tracksCapacity: number
  tracksLen: number
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
    fill: { ms: number; failed: number; queued: number }
    counters: {
      prepare: { allocs: number; frees: number }
      render: { allocs: number; frees: number }
      busy: number
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
    const clockBuffer = new SharedArrayBuffer(4)
    const clock = new Int32Array(clockBuffer)
    const clockWorker = new Worker(blob(sources.clock))
    clockWorker.postMessage(clockBuffer)
    const calibrate = async () => {
      const c0 = Atomics.load(clock, 0)
      const t0 = performance.now()
      await sleep(300)
      const c1 = Atomics.load(clock, 0)
      const t1 = performance.now()
      return ((c1 - c0) >>> 0) / ((t1 - t0) * 1000)
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

    // Load: ten bursts of 100 messages; then the track table grown to its limit (4096) and
    // filled with 4096 boxed tracks; then three growths, each a million-note snapshot plus
    // the slot array doubled to 8192 (the 4096 slots moved in apply, then 8192 for the two
    // re-swaps), all applied while the note keeps playing.
    await ask({ type: 'reset' })
    const workerBursts: Record<string, unknown>[] = []
    const workerGrows: Record<string, unknown>[] = []
    let workerFill: Record<string, unknown> = {}
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
    if (mode === 'a') {
      node.port.postMessage({ type: 'grow', notes: 0, tracks: 4096 })
      node.port.postMessage({ type: 'fill', n: 4096 })
    } else {
      workerGrows.push(await askWorker!({ type: 'grow', notes: 0, tracks: 4096 }))
      workerFill = await askWorker!({ type: 'fill', n: 4096 })
      // 4096 swaps drain at 128 per block: about 32 blocks.
      await sleep(300)
    }
    for (let i = 0; i < 3; i++) {
      // The first doubling moves the 4096 entries; the next two re-swap the 8192-slot array
      // (8192 slots moved, twice the limit: conservative for the budget).
      const tracks = 8192
      if (mode === 'a') node.port.postMessage({ type: 'grow', notes: 1_000_000, tracks })
      else workerGrows.push(await askWorker!({ type: 'grow', notes: 1_000_000, tracks }))
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
          ? ({ bursts: workerBursts, grows: workerGrows, fill: workerFill, counters, maximumPages } as unknown as Run['worker'])
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
    for (const f of load.fills) lines.push(`  fill of ${f.n} tracks on the render thread: ${us(f.us)} µs (failed ${f.failed}, pumped ${f.pumped})`)
    for (const [i, g] of load.grows.entries())
      lines.push(
        `  growth on the render thread: ${us(g.us)} µs (${i === 0 ? 'empty table to 4096 slots' : '1 M notes + the slot array re-swapped (4096 then 8192 slots moved)'}, pump ${g.pumped}, collect ${g.collected}), memory ${g.before >> 20} → ${g.after >> 20} MiB, buffer detached ${g.detached}`,
      )
    lines.push(`  render-side allocations ${load.renderAllocs}, frees ${load.renderFrees} (prepare runs there in this way; the one-time setup is not counted)`)
  } else {
    lines.push(
      `  per message on the render thread: pump ${us(load.pumpUsPerMsg)} µs/msg (${load.pumped} messages in ${load.pumpCalls} pumps, ${us(load.pumpUsPerCall)} µs per pump incl. empty ones, longest pump ${us(load.pumpMaxUs)} µs incl. the swaps; idle pump ${us(idle.pumpUsPerCall)} µs)`,
    )
    lines.push(`  fill of 4096 tracks: prepare in the worker ${run.worker!.fill.ms.toFixed(2)} ms (failed ${run.worker!.fill.failed}, queued behind a full ring ${run.worker!.fill.queued})`)
    for (const [i, g] of run.worker!.grows.entries())
      lines.push(
        `  growth: prepare in the worker ${g.ms.toFixed(2)} ms (${i === 0 ? 'empty table to 4096 slots' : '1 M notes + the slot array re-swapped (4096 then 8192 slots moved in the pump)'}, codes ${g.notes}/${g.tracks}), memory ${g.before >> 20} → ${g.after >> 20} MiB; worker burst ${i < run.worker!.bursts.length ? run.worker!.bursts[i].ms.toFixed(3) : '-'} ms/100`,
      )
    const c = run.worker!.counters
    lines.push(
      `  render-side allocations ${c.render.allocs}, frees ${c.render.frees}; prepare-side ${c.prepare.allocs}/${c.prepare.frees}; ring busy ${c.busy}; maximum ${run.worker!.maximumPages} pages; roles worker ${c.role} / worklet ${run.workletRole}`,
    )
  }
  lines.push(
    `  views re-taken ${load.viewsRefreshed} times, memory ${load.memoryBytes >> 20} MiB, notes table ${load.notes}, tracks ${load.tracksLen} in ${load.tracksCapacity} slots, applied ${load.applied}, skipped ${load.skipped}, garbage refused ${load.garbageRefused}`,
  )
  return lines.join('\n')
}

test.describe('CT-3: the project → engine channel in the browser', () => {
  test('way (a), plain memory: messages and growth on the render thread', async ({ page, baseURL }) => {
    await serveProto(page, baseURL!)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
    const run = await measure(page, 'a')
    console.log(`timing: ceiling only (10x: ${ceiling(2)} µs per message, ${ceiling(500)} µs longest pump, ${ceiling(50)} µs per message in a burst), in every mode`)
    console.log(table(run))
    const { load } = run
    expect(run.ticksPerUs).toBeGreaterThan(1)
    expect(load.msgCount).toBe(1000)
    expect(load.peak).toBeGreaterThan(0.01)
    expect(load.grows).toHaveLength(4)
    for (const g of load.grows) {
      expect(g.notes).toBe(0)
      expect(g.tracks).toBe(0)
      expect(g.pumped).toBe(2)
    }
    expect(load.fills).toEqual([expect.objectContaining({ n: 4096, failed: 0, pumped: 4096 })])
    expect(load.tracksLen).toBe(4096)
    expect(load.skipped).toBe(0)
    expect(load.garbageRefused).toBe(0)
    // Attack: memory.grow while the worklet holds views. A plain memory detaches the old
    // buffer; the views are re-taken at the next block and the audio goes on.
    expect(load.grows.some((g) => g.detached)).toBe(true)
    expect(load.viewsRefreshed).toBeGreaterThanOrEqual(1)
    expect(load.notes).toBe(1_000_000)
    expect(load.tracksCapacity).toBe(8192)
    // A message costs under 500 µs of render thread, dispatch included (ceiling of a 50 µs budget).
    for (const b of load.bursts) expect(b.usPerMsg).toBeLessThan(ceiling(50))
    // Prepare on the render thread is the price of this way: it allocates and frees there.
    expect(load.renderAllocs).toBeGreaterThan(0)
  })

  test('way (b), shared memory: a Worker prepares, the worklet pumps', async ({ page, baseURL }) => {
    await serveProto(page, baseURL!)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
    const run = await measure(page, 'b')
    console.log(`timing: ceiling only (10x: ${ceiling(2)} µs per message, ${ceiling(500)} µs longest pump, ${ceiling(50)} µs per message in a burst), in every mode`)
    console.log(table(run))
    const { load } = run
    const c = run.worker!.counters
    expect(run.ticksPerUs).toBeGreaterThan(1)
    // Two instances on one memory: the worker is the prepare side, the worklet the render side,
    // each with its own thread-local role and its own stack.
    expect(c.role).toBe(1)
    expect(run.workletRole).toBe(2)
    // 1000 parameters, the growth to 4096 slots (two swaps: an empty notes table and the
    // slot array), 4096 tracks, three growths of two swaps.
    expect(load.pumped).toBe(1000 + 2 + 4096 + 6)
    expect(load.applied).toBe(1000 + 2 + 4096 + 6)
    // What the full ring refused during the fill waited in the worker's queue and went through.
    expect(c.busy).toBeGreaterThanOrEqual(run.worker!.fill.queued)
    expect(run.worker!.fill.failed).toBe(0)
    expect(load.tracksLen).toBe(4096)
    expect(load.skipped).toBe(0)
    expect(load.garbageRefused).toBe(0)
    expect(load.peak).toBeGreaterThan(0.01)
    // The render side never allocated nor freed: the growths were prepared in the worker and
    // the old tables dropped there.
    expect(load.renderAllocs).toBe(0)
    expect(load.renderFrees).toBe(0)
    expect(c.prepare.frees).toBeGreaterThan(0)
    expect(load.notes).toBe(1_000_000)
    expect(load.tracksCapacity).toBe(8192)
    expect(run.worker!.grows).toHaveLength(4)
    for (const g of run.worker!.grows) {
      expect(g.notes).toBe(0)
      expect(g.tracks).toBe(0)
      expect(g.after).toBeGreaterThanOrEqual(g.before)
    }
    expect(run.worker!.grows[1].after).toBeGreaterThan(run.worker!.grows[1].before)
    // The budget for the pump that swaps a runtime table at its limit (4096, then 8192
    // slots moved): under 500 µs (ceiling, see support.ts).
    expect(load.pumpMaxUs).toBeLessThan(ceiling(500))
    // Attack: memory.grow while the worklet holds views. A shared memory keeps the old buffer
    // valid; the worklet re-took its views (memory.buffer changed) and the audio went on.
    expect(load.viewsRefreshed).toBeGreaterThanOrEqual(1)
    expect(load.silentBlocks).toBe(0)
    // The budget of § 2.3 (2 µs per pumped message; ceiling, see support.ts).
    expect(load.pumpUsPerMsg).toBeLessThan(ceiling(2))
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
