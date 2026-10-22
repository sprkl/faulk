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

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url))
const STREAM = process.env.FAULK_BENCH_STREAM ?? ''

/** The fixture format of crates/engine-wasm/tests/parity.rs: the pushes, in order. */
function pushes(path: string): Buffer[] {
  const bytes = readFileSync(path)
  const out: Buffer[] = []
  for (let at = 0; at + 5 <= bytes.length; ) {
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
        async ({ sources, count }) => {
          const blob = (s: string) => URL.createObjectURL(new Blob([s], { type: 'text/javascript' }))
          const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
          const isolated = crossOriginIsolated
          const clockBuffer = new SharedArrayBuffer(4)
          const clock = new Int32Array(clockBuffer)
          const clockWorker = new Worker(blob(sources.clock))
          clockWorker.postMessage(clockBuffer)
          await sleep(100)
          const c0 = Atomics.load(clock, 0)
          const t0 = performance.now()
          await sleep(300)
          const ticksPerUs = ((Atomics.load(clock, 0) - c0) >>> 0) / ((performance.now() - t0) * 1000)

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
          await context.resume()
          const worker = new Worker(blob(sources.prepare))
          const fromWorker: ((m: Record<string, unknown>) => void)[] = []
          worker.onmessage = (e) => {
            if (e.data.type === 'error') throw new Error(`prepare: ${e.data.message}`)
            fromWorker.shift()?.(e.data)
          }
          const askWorker = (m: Record<string, unknown>) =>
            new Promise<Record<string, unknown>>((resolve) => {
              fromWorker.push(resolve)
              worker.postMessage(m)
            })
          const ready = await askWorker({ type: 'init', bytes: bytes.slice(0), memory, sampleRate: context.sampleRate })
          await context.audioWorklet.addModule(blob(sources.worklet))
          const node = new AudioWorkletNode(context, 'faulk-measure', { numberOfInputs: 0, outputChannelCount: [2] })
          const fromWorklet: ((m: Record<string, unknown>) => void)[] = []
          node.port.onmessage = (e) => {
            if (e.data.type === 'error') throw new Error(`worklet: ${e.data.message}`)
            fromWorklet.shift()?.(e.data)
          }
          const ask = (m: Record<string, unknown>) =>
            new Promise<Record<string, unknown>>((resolve) => {
              fromWorklet.push(resolve)
              node.port.postMessage(m)
            })
          await ask({ type: 'init', bytes, memory, setup: ready.setup, clock: clockBuffer, ticksPerUs })
          node.connect(context.destination)
          await sleep(500)
          const idle = (await ask({ type: 'report' })) as unknown as Record<string, number>
          const memBefore = memory!.buffer.byteLength

          // Cut the stream into its pushes again (the file format: 0, u32 length, bytes).
          const view = new DataView(stream.buffer)
          const pushes: ArrayBuffer[] = []
          for (let at = 0; at + 5 <= stream.length; ) {
            const n = view.getUint32(at + 1, true)
            if (stream[at] === 0) pushes.push(stream.slice(at + 5, at + 5 + n).buffer)
            at += 5 + (stream[at] === 0 ? n : 0)
          }

          await ask({ type: 'reset' })
          const start = performance.now()
          await askWorker({ type: 'push', streams: pushes })
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
          const swap = (await ask({ type: 'report' })) as unknown as Record<string, number>
          const counters = await askWorker({ type: 'counters' })
          clockWorker.terminate()
          worker.terminate()
          await context.close()
          return { isolated, ticksPerUs, idle, swap, counters, done, memBefore, memAfter: memory!.buffer.byteLength, pushes: pushes.length, count, sampleRate: context.sampleRate }
        },
        { sources, count: all.length },
      )
      const us = (n: number) => n.toFixed(2)
      const s = result.swap
      console.log(
        [
          `QA-7 engine swaps in the browser: ${result.pushes} pushes of the heavy project, ${(readFileSync(STREAM).length / 1048576).toFixed(1)} MiB, at ${result.sampleRate} Hz, clock ${result.ticksPerUs.toFixed(1)} ticks/µs, cross-origin isolated ${result.isolated}`,
          `  prepare and apply complete in ${result.done.toFixed(0)} ms wall from the first push to an empty queue (the worklet applies at most 128 messages per block)`,
          `  worklet: ${s.applied} messages applied over ${s.blocks} blocks (at most ${s.maxApplied} per block), pump ${us(s.pumpUsPerMsg)} µs per message, longest pump ${us(s.pumpMaxUs)} µs, ${us(s.pumpUsPerCall)} µs per pump incl. empty ones; skipped ${s.skipped}; render-side allocations ${s.renderAllocs}, frees ${s.renderFrees}; silent blocks ${s.silent}`,
          `  prepare side: rejected ${result.counters.rejected}, codes ${JSON.stringify(result.counters.codes)}; tracks ${s.tracksHeld} in ${s.tracksCapacity} slots`,
          `  shared linear memory: ${(result.memBefore / 1048576).toFixed(0)} MiB before, ${(result.memAfter / 1048576).toFixed(0)} MiB after the whole project`,
        ].join('\n'),
      )
      expect(result.isolated).toBe(true)
      expect(result.done).toBeGreaterThan(0)
      // The whole stream was applied: the worklet's count equals the engine's own, and the
      // number of messages the native bench printed when set (2403 for the shipped stream).
      expect(s.applied).toBe(s.hostApplied)
      if (process.env.FAULK_BENCH_EXPECT_APPLIED) expect(s.applied).toBe(Number(process.env.FAULK_BENCH_EXPECT_APPLIED))
      expect(result.counters.rejected).toBe(0)
      expect(result.counters.codes).toEqual([])
      expect(s.skipped).toBe(0)
      expect(s.tracksHeld).toBe(100)
      expect(s.renderAllocs).toBe(0)
      expect(s.renderFrees).toBe(0)
      expect(errors).toEqual([])
    } finally {
      server.close()
    }
  })
})
