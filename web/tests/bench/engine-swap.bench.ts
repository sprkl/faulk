import { createServer } from 'node:http'
import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test } from '@playwright/test'

// QA-7 (#130): the engine's structure swaps (docs/specs/02-decoupage-mvp.md § 2.3) on the heavy
// project, in the shipped engine module and a real AudioWorklet. The load is the heavy project's
// engine stream (100 tracks, 200 000 notes, 6 million automation points), written by the native
// bench (`load stream --out <file>`, crates/project/benches/load): the same messages the page
// would send. It goes through the prepare Worker into the shared memory and is applied by the
// worklet's pump, as in `tests/e2e/engine-mo2.spec.ts`, whose harness this reuses. Run by hand:
//
//   FAULK_BENCH_STREAM=<file> pnpm bench:project engine-swap
//
// A measurement, not a test: numbers are printed, the assertions prove the queue drained and the
// stream was applied whole (count equal to the engine's own, nothing rejected, nothing skipped). The budget of MO-2 for a pump is 500 µs.
// The first pump that applies anything is printed apart: it runs wasm code for the first time (V8 compiles a function at its first
// call, #283). `FAULK_BENCH_WARM=<n>` sends the first n pushes before the measured window to show what a warm-up buys (the count
// assertions are then skipped); `FAULK_BENCH_JSFLAGS` passes V8 flags to Chromium (see playwright.bench.config.ts).

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url))
const STREAM = process.env.FAULK_BENCH_STREAM ?? ''
const WARM = Number(process.env.FAULK_BENCH_WARM ?? 0)
// `FAULK_BENCH_WARMUP=0` skips the engine's own warm-up of the apply path (#355): the cold start it removes.
const WARMUP = process.env.FAULK_BENCH_WARMUP !== '0'

/** The fixture format of crates/engine-wasm/tests/parity.rs: the pushes, in order. */
function pushes(path: string): Buffer[] {
  const bytes = readFileSync(path)
  const out: Buffer[] = []
  for (let at = 0; at + 5 <= bytes.length;) {
    const n = bytes.readUInt32LE(at + 1)
    if (bytes[at] === 0) out.push(bytes.subarray(at + 5, at + 5 + n))
    at += 5 + (bytes[at] === 0 ? n : 0)
  }
  return out
}

test.describe('QA-7: the heavy project through the engine channel in the browser', () => {
  test.skip(!STREAM || !existsSync(STREAM), 'set FAULK_BENCH_STREAM (see the header of this file)')

  test('prepare in a Worker, apply in the worklet: the cost of the swaps', async ({ page }) => {
    test.setTimeout(10 * 60_000)
    const wasm = readFileSync(here('../../src/lib/engine/wasm/engine.wasm'))
    const sources = {
      clock: readFileSync(here('../../src/lib/engine/proto/clock-worker.js'), 'utf8'),
      prepare: readFileSync(here('../e2e/fixtures/measure-prepare.js'), 'utf8'),
      worklet: readFileSync(here('../e2e/fixtures/measure-worklet.js'), 'utf8'),
      rpc: readFileSync(here('../e2e/fixtures/measure-rpc.js'), 'utf8'),
    }
    const all = pushes(STREAM)
    // Cross-origin isolation (shared memory), a blank page, the stream and the module.
    const server = createServer((req, res) => {
      const headers = { 'cross-origin-opener-policy': 'same-origin', 'cross-origin-embedder-policy': 'require-corp', 'cache-control': 'no-store' }
      if (req.url === '/stream') res.writeHead(200, { ...headers, 'content-type': 'application/octet-stream' }).end(readFileSync(STREAM))
      else if (req.url === '/engine.wasm') res.writeHead(200, { ...headers, 'content-type': 'application/wasm' }).end(wasm)
      else res.writeHead(200, { ...headers, 'content-type': 'text/html' }).end('<!doctype html><title>qa7</title>')
    })
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
    const origin = `http://localhost:${(server.address() as { port: number }).port}`
    const errors: string[] = []
    page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
    page.on('pageerror', (e) => errors.push(String(e)))
    try {
      await page.goto(origin + '/')
      const result = await page.evaluate(
        async ({ sources, count, warm, warmup }) => {
          const blob = (s: string) => URL.createObjectURL(new Blob([s], { type: 'text/javascript' }))
          const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
          const isolated = crossOriginIsolated
          const clockBuffer = new SharedArrayBuffer(4)
          const clock = new Int32Array(clockBuffer)
          const clockWorker = new Worker(blob(sources.clock))
          clockWorker.postMessage(clockBuffer)
          const rpc = new Function(`return (${sources.rpc}
)`)() as (target: unknown, name: string, ms: number) => ((m: Record<string, unknown>) => Promise<Record<string, unknown>>) & { fail(error: Error): void }
          const deadline = <T>(what: string, promise: Promise<T>, ms = 15_000) =>
            Promise.race([promise, new Promise<never>((_, reject) => setTimeout(() => reject(new Error(`${what}: no answer within ${ms} ms`)), ms))])
          // The counter's rate over a window (the Worker that spins is slow for its first few hundred ms, #283).
          const mark = () => ({ c: Atomics.load(clock, 0), t: performance.now() })
          const rateSince = (m: { c: number; t: number }) => ((Atomics.load(clock, 0) - m.c) >>> 0) / ((performance.now() - m.t) * 1000)
          const calibrate = async (ms: number) => {
            const m = mark()
            await sleep(ms)
            return rateSince(m)
          }
          const settle = async () => {
            const rates = [await calibrate(200)]
            for (let i = 0; i < 40; i++) {
              rates.push(await calibrate(200))
              const [a, b, c] = rates.slice(-3)
              // At least a second, and the last three windows within 10 % of each other.
              if (rates.length >= 5 && Math.max(a, b, c) < 1.1 * Math.min(a, b, c)) return c
            }
            throw new Error('clock: the counter rate never settled')
          }
          await sleep(100)
          const ticksPerUs = await settle()

          let memory: WebAssembly.Memory | undefined
          for (const maximum of [65536, 32768, 16384]) {
            try {
              memory = new WebAssembly.Memory({ initial: 256, maximum, shared: true })
              break
            } catch {
              /* a smaller maximum */
            }
          }
          const bytes = await (await fetch('/engine.wasm')).arrayBuffer()
          const stream = new Uint8Array(await (await fetch('/stream')).arrayBuffer())
          const context = new AudioContext()
          await deadline(`audio context (state ${context.state})`, context.resume())
          const worker = new Worker(blob(sources.prepare))
          const askWorker = rpc(worker, 'prepare Worker', 60_000)
          const ready = await askWorker({ type: 'init', bytes: bytes.slice(0), memory, sampleRate: context.sampleRate })
          await context.audioWorklet.addModule(blob(sources.worklet))
          const node = new AudioWorkletNode(context, 'faulk-measure', { numberOfInputs: 0, outputChannelCount: [2] })
          const ask = rpc(node.port, 'worklet', 15_000)
          // A trap inside `process` is reported on the node, not on its port.
          node.onprocessorerror = () => ask.fail(new Error('worklet: processorerror (an exception in process)'))
          let since = mark()
          const reset = async () => {
            await ask({ type: 'reset' })
            since = mark()
          }
          // The report is converted at the counter's rate over the window the reset opened.
          const report = async () => {
            const rate = rateSince(since)
            await ask({ type: 'rate', ticksPerUs: rate })
            return { ...(await ask({ type: 'report' })), rate } as unknown as Record<string, number>
          }
          const initAt = performance.now()
          const started = await ask({ type: 'init', bytes, memory, setup: ready.setup, clock: clockBuffer, ticksPerUs, warm: warmup })
          const initMs = performance.now() - initAt
          if (warmup && started.warm !== 0) throw new Error(`the warm-up failed: ${String(started.warm)}`)
          node.connect(context.destination)
          await sleep(500)
          await reset()
          await sleep(500)
          const idle = await report()
          const memBefore = memory!.buffer.byteLength

          // Cut the stream into its pushes again (the file format: 0, u32 length, bytes).
          const view = new DataView(stream.buffer)
          const pushes: ArrayBuffer[] = []
          for (let at = 0; at + 5 <= stream.length;) {
            const n = view.getUint32(at + 1, true)
            if (stream[at] === 0) pushes.push(stream.slice(at + 5, at + 5 + n).buffer)
            at += 5 + (stream[at] === 0 ? n : 0)
          }

          if (warm > 0) {
            await askWorker({ type: 'push', streams: pushes.slice(0, warm) })
            await sleep(500)
          }
          await reset()
          const start = performance.now()
          await askWorker({ type: 'push', streams: pushes.slice(warm) })
          let done = 0
          for (let i = 0; i < 600; i++) {
            const c = await askWorker({ type: 'counters' })
            if (c.queued === 0 && c.pending === 0) {
              done = performance.now() - start
              break
            }
            await sleep(50)
          }
          await sleep(300)
          const swap = await report()
          // The rate again after the load: the counter must have kept its pace (#283).
          const check = await calibrate(300)
          const counters = await askWorker({ type: 'counters' })
          clockWorker.terminate()
          worker.terminate()
          await deadline(`closing the audio context (state ${context.state})`, context.close())
          return {
            initMs,
            isolated,
            ticksPerUs,
            check,
            idle,
            swap,
            counters,
            done,
            memBefore,
            memAfter: memory!.buffer.byteLength,
            pushes: pushes.length,
            count,
            sampleRate: context.sampleRate,
          }
        },
        { sources, count: all.length, warm: WARM, warmup: WARMUP },
      )
      const us = (n: number) => n.toFixed(2)
      const s = result.swap
      console.log(
        [
          `QA-7 engine swaps in the browser: ${result.pushes} pushes of the heavy project, ${(readFileSync(STREAM).length / 1048576).toFixed(1)} MiB, at ${result.sampleRate} Hz, clock ${result.ticksPerUs.toFixed(1)} ticks/µs (window ${s.rate.toFixed(1)}, check ${result.check.toFixed(1)}), cross-origin isolated ${result.isolated}, worklet init (instantiate${WARMUP ? ' and warm-up' : ''}) ${result.initMs.toFixed(1)} ms`,
          `  prepare and apply complete in ${result.done.toFixed(0)} ms wall from the first push to an empty queue (the worklet applies at most 128 messages per block)`,
          `  worklet: ${s.applied} messages applied over ${s.blocks} blocks (at most ${s.maxApplied} per block), pump ${us(s.pumpUsPerMsg)} µs per message, longest pump ${us(s.pumpMaxUs)} µs after the first one, which cost ${us(s.firstUs)} µs for ${s.firstApplied} messages (V8 compiles a wasm function on its first call), ${us(s.pumpUsPerCall)} µs per pump incl. empty ones; skipped ${s.skipped}; render-side allocations ${s.renderAllocs}, frees ${s.renderFrees}; silent blocks ${s.silent}`,
          `  prepare side: rejected ${String(result.counters.rejected)}, codes ${JSON.stringify(result.counters.codes)}; tracks ${s.tracksHeld} in ${s.tracksCapacity} slots`,
          `  shared linear memory: ${(result.memBefore / 1048576).toFixed(0)} MiB before, ${(result.memAfter / 1048576).toFixed(0)} MiB after the whole project`,
        ].join('\n'),
      )
      expect(result.isolated).toBe(true)
      expect(result.done).toBeGreaterThan(0)
      // The counter kept its pace between the calibration and the end (#283), or every figure is off.
      expect(Math.abs(s.rate - result.check)).toBeLessThan(0.25 * result.check)
      expect(Math.abs(result.ticksPerUs - result.check)).toBeLessThan(0.25 * result.check)
      // The whole stream was applied: the worklet's count equals the engine's own, and the
      // number of messages the native bench printed when set (2403 for the shipped stream).
      if (!WARM) expect(s.applied).toBe(s.hostApplied)
      if (!WARM && process.env.FAULK_BENCH_EXPECT_APPLIED) expect(s.applied).toBe(Number(process.env.FAULK_BENCH_EXPECT_APPLIED))
      expect(result.counters.rejected).toBe(0)
      expect(result.counters.codes).toEqual([])
      expect(s.skipped).toBe(0)
      // Opt-in strict figure (a quiet reference machine): the first applying pump under the given µs (#355).
      if (process.env.FAULK_BENCH_FIRST_MAX_US) expect(s.firstUs).toBeLessThan(Number(process.env.FAULK_BENCH_FIRST_MAX_US))
      expect(s.tracksHeld).toBe(100)
      expect(s.renderAllocs).toBe(0)
      expect(s.renderFrees).toBe(0)
      expect(errors).toEqual([])
    } finally {
      server.close()
    }
  })
})
