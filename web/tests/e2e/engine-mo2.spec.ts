import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { STRICT_RUNS, STRICT_TIMING, fakeWebMidi, median, timingLimit, timingMode, watch } from './support'

// MO-2 (#55): the shipped engine channel in a real AudioWorklet in Chromium. The shipped module
// (src/lib/engine/wasm/engine.wasm, wasm32-wasip1-threads) runs as two instances on one shared
// memory: a Worker prepares, the worklet pumps at the start of each block. The load is the Rust
// recording of crates/engine-wasm/tests/fixtures (nothing is encoded here): the setup and a held
// note, 1 000 parameter messages, then the track table filled to its limit (4 095 tracks, its
// last growth moving 2 048 entries in the pump). Numbers are printed for the pull request; the
// assertions prove the messages were applied and hold CT-3's budget (2 µs per pumped message;
// 500 µs for the pump that swaps the table at its limit) by a ceiling ten times higher in CI, and
// by the budget itself on the median of five runs with FAULK_E2E_STRICT_TIMING=1 (support.ts).

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url))
const fixtures = here('../../../crates/engine-wasm/tests/fixtures/')

test.describe.configure({ mode: 'serial' })

/** The fixture format of crates/engine-wasm/tests/parity.rs: the pushes, in order. */
function pushes(name: string): number[][] {
  const bytes = readFileSync(fixtures + name)
  const out: number[][] = []
  for (let at = 0; at + 5 <= bytes.length; ) {
    const n = bytes.readUInt32LE(at + 1)
    if (bytes[at] === 0) {
      out.push([...bytes.subarray(at + 5, at + 5 + n)])
      at += 5 + n
    } else at += 5
  }
  return out
}

type Report = {
  blocks: number
  silent: number
  applied: number
  maxApplied: number
  pumpUsPerMsg: number
  pumpUsPerCall: number
  pumpMaxUs: number
  firstUs: number
  rate: number
  views: number
  peak: number
  renderAllocs: number
  renderFrees: number
  hostApplied: number
  skipped: number
  tracksCapacity: number
  tracksHeld: number
}

async function measure(page: Page) {
  const session = pushes('session.stream')
  const sources = {
    clock: readFileSync(here('../../src/lib/engine/proto/clock-worker.js'), 'utf8'),
    prepare: readFileSync(here('fixtures/measure-prepare.js'), 'utf8'),
    worklet: readFileSync(here('fixtures/measure-worklet.js'), 'utf8'),
    rpc: readFileSync(here('fixtures/measure-rpc.js'), 'utf8'),
  }
  const load = {
    // Before the first render: the setup, a waveform, the held note (the session's start).
    start: session.slice(0, 3),
    // One parameter message of the session (a waveform), sent 100 times per burst.
    param: session[5],
    tracks: pushes('tracks.stream'),
    wasm: [...readFileSync(here('../../src/lib/engine/wasm/engine.wasm'))],
  }
  return page.evaluate(
    async ({ sources, load }) => {
      const blob = (s: string) => URL.createObjectURL(new Blob([s], { type: 'text/javascript' }))
      const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
      const u8 = (a: number[]) => new Uint8Array(a).buffer
      const clockBuffer = new SharedArrayBuffer(4)
      const clock = new Int32Array(clockBuffer)
      const clockWorker = new Worker(blob(sources.clock))
      clockWorker.postMessage(clockBuffer)
      const rpc = new Function(`return (${sources.rpc}
)`)() as (target: unknown, name: string, ms: number) => ((m: Record<string, unknown>) => Promise<Record<string, unknown>>) & { fail(error: Error): void }
      const deadline = <T,>(what: string, promise: Promise<T>, ms = 15_000) =>
        Promise.race([promise, new Promise<never>((_, reject) => setTimeout(() => reject(new Error(`${what}: no answer within ${ms} ms`)), ms))])
      // The counter's rate over a window: ticks gained divided by the µs `performance.now()` counted.
      const mark = () => ({ c: Atomics.load(clock, 0), t: performance.now() })
      const rateSince = (m: { c: number; t: number }) => ((Atomics.load(clock, 0) - m.c) >>> 0) / ((performance.now() - m.t) * 1000)
      const calibrate = async (ms = 300) => {
        const m = mark()
        await sleep(ms)
        return rateSince(m)
      }
      // The spinning Worker is slow for its first few hundred ms (baseline tier, about six times
      // below its later rate, for 200 to 800 ms, #283): wait until three windows agree before using its rate.
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
      const bytes = u8(load.wasm)
      const context = new AudioContext()
      await deadline(`audio context (state ${context.state})`, context.resume())
      const worker = new Worker(blob(sources.prepare))
      const askWorker = rpc(worker, 'prepare Worker', 15_000)
      const ready = await askWorker({ type: 'init', bytes: bytes.slice(0), memory, sampleRate: context.sampleRate })

      await context.audioWorklet.addModule(blob(sources.worklet))
      const node = new AudioWorkletNode(context, 'faulk-measure', { numberOfInputs: 0, outputChannelCount: [2] })
      const ask = rpc(node.port, 'worklet', 15_000)
      // A trap inside `process` is reported on the node, not on its port.
      node.onprocessorerror = () => ask.fail(new Error('worklet: processorerror (an exception in process)'))
      // A reset opens a measured window; its report is converted at the counter's rate over that window.
      let since = mark()
      const reset = async () => {
        await ask({ type: 'reset' })
        since = mark()
      }
      const report = async () => {
        const rate = rateSince(since)
        await ask({ type: 'rate', ticksPerUs: rate })
        return { ...(await ask({ type: 'report' })), rate } as unknown as Record<string, number>
      }
      const role = (await ask({ type: 'init', bytes, memory, setup: ready.setup, clock: clockBuffer, ticksPerUs })).role
      node.connect(context.destination)

      await askWorker({ type: 'push', streams: load.start.map(u8) })
      await sleep(300)
      await reset()
      await sleep(1000)
      const idle = await report()

      await reset()
      for (let burst = 0; burst < 10; burst++) {
        await askWorker({ type: 'push', streams: Array.from({ length: 100 }, () => u8(load.param)) })
        await sleep(50)
      }
      await sleep(200)
      const params = await report()

      await reset()
      await askWorker({ type: 'push', streams: load.tracks.map(u8) })
      // 4 095 messages drain at 128 per block (about 32 blocks); the Worker resends what
      // the full rings refused.
      for (let i = 0; i < 40; i++) {
        const c = await askWorker({ type: 'counters' })
        if (c.queued === 0 && c.pending === 0) break
        await sleep(100)
      }
      await sleep(300)
      const tracks = await report()
      const counters = await askWorker({ type: 'counters' })
      const check = await calibrate()
      clockWorker.terminate()
      worker.terminate()
      await deadline(`closing the audio context (state ${context.state})`, context.close())
      return { ticksPerUs, check, role, idle, params, tracks, counters, sampleRate: context.sampleRate, maximum: memory!.buffer.byteLength }
    },
    { sources, load },
  )
}

test.describe('MO-2: the engine channel in the browser', () => {
  test.skip(({ browserName }) => Boolean(browserName) && test.info().project.name !== 'dev', 'measured once, under the dev project')

  test('a Worker prepares, the worklet pumps: cost per message within the budget', async ({ page }) => {
    await page.goto('/app')
    test.setTimeout(STRICT_TIMING ? 180_000 : 30_000)
    // On a reference machine the timing is the median of several runs, each its own full
    // measurement; every run is printed and checked for correctness, not only the first.
    const runs = [await measure(page)]
    if (STRICT_TIMING) for (let i = 1; i < STRICT_RUNS; i++) runs.push(await measure(page))
    const us = (n: number) => n.toFixed(2)
    console.log(timingMode([2, 500]))
    for (const [i, run] of runs.entries()) {
      const p = run.params as unknown as Report
      const t = run.tracks as unknown as Report
      console.log(
        [
          `MO-2 run ${i + 1}/${runs.length} at ${run.sampleRate} Hz, clock ${run.ticksPerUs.toFixed(1)} ticks/µs (check ${run.check.toFixed(1)})`,
          `  idle: ${run.idle.blocks} blocks, pump ${us(run.idle.pumpUsPerCall)} µs per call`,
          `  1000 parameters: ${p.applied} applied, ${us(p.pumpUsPerMsg)} µs per message (pumps that applied), longest pump ${us(Math.max(p.pumpMaxUs, p.firstUs))} µs (first ${us(p.firstUs)}), ${us(p.pumpUsPerCall)} µs per pump incl. empty ones, silent blocks ${p.silent}, peak ${p.peak.toFixed(3)}`,
          `  4095 tracks: ${t.applied} applied (at most ${t.maxApplied} per block), ${us(t.pumpUsPerMsg)} µs per message, longest pump ${us(Math.max(t.pumpMaxUs, t.firstUs))} µs (first ${us(t.firstUs)}; incl. the swap of the table at its limit), tracks ${t.tracksHeld} in ${t.tracksCapacity} slots, silent ${t.silent}`,
          `  render-side allocations ${t.renderAllocs + p.renderAllocs}, frees ${t.renderFrees + p.renderFrees}; rejected ${run.counters.rejected}, codes ${JSON.stringify(run.counters.codes)}, skipped ${t.skipped}; roles worklet ${run.role}`,
        ].join('\n'),
      )
      expect(run.ticksPerUs).toBeGreaterThan(1)
      // The clock must not have changed pace between its calibration and the end (#283): a
      // rate off by more than 25 % would make every figure above wrong by as much.
      expect(Math.abs(run.check - run.ticksPerUs)).toBeLessThan(0.25 * run.check)
      for (const r of [p, t]) expect(Math.abs(r.rate - run.check)).toBeLessThan(0.25 * run.check)
      expect(run.role).toBe(2)
      // Every message was applied: none rejected, none skipped, the table at its limit.
      expect(run.counters.rejected).toBe(0)
      expect(run.counters.codes).toEqual([])
      expect(p.applied).toBe(1000)
      expect(t.applied).toBe(4095)
      expect(t.tracksHeld).toBe(4096)
      expect(t.tracksCapacity).toBe(4096)
      expect(t.skipped).toBe(0)
      expect(t.maxApplied).toBeLessThanOrEqual(128)
      // No allocation nor free on the render instance; the note never fell silent.
      expect(p.renderAllocs + t.renderAllocs).toBe(0)
      expect(p.renderFrees + t.renderFrees).toBe(0)
      expect(p.silent).toBe(0)
      expect(t.silent).toBe(0)
      expect(p.peak).toBeGreaterThan(0.01)
    }
    // The budget of CT-3: 2 µs per message, 500 µs for the longest pump. The ceiling by default,
    // the budget itself on the median of the runs when strict (timingLimit).
    const reports = runs.map((r) => ({ p: r.params as unknown as Report, t: r.tracks as unknown as Report }))
    const perParam = median(reports.map((r) => r.p.pumpUsPerMsg))
    const perTrack = median(reports.map((r) => r.t.pumpUsPerMsg))
    // The longest pump counts the first one that applied anything too: the engine is warm here (the
    // setup went through before the window), so it is not a cold start.
    const longest = median(reports.map((r) => Math.max(r.t.pumpMaxUs, r.t.firstUs)))
    console.log(`asserted (median of ${runs.length}): ${us(perParam)} / ${us(perTrack)} µs per message, longest pump ${us(longest)} µs`)
    expect(perParam).toBeLessThan(timingLimit(2))
    expect(perTrack).toBeLessThan(timingLimit(2))
    expect(longest).toBeLessThan(timingLimit(500))
  })

  test('the DAW plays a note and takes a parameter through the channel', async ({ page }) => {
    await fakeWebMidi(page)
    const { errors } = watch(page)
    // The warm-up of the apply path (#355) must go through in the real worklet (it warns when it fails).
    const warnings: string[] = []
    page.on('console', (m) => m.type() === 'warning' && warnings.push(m.text()))
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
    await expect(page.getByTestId('engine-mode')).toContainText('Web')
    expect(warnings.filter((w) => w.includes('warm-up'))).toEqual([])
    await page.getByRole('radio', { name: 'E2E Keys' }).check()
    const meter = page.getByRole('meter').first()
    const level = async () => Number(await meter.getAttribute('aria-valuenow'))
    await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x90, 69, 127]))
    await expect(page.getByText('Voices: 1')).toBeVisible()
    await expect.poll(level).toBeGreaterThan(-20)
    const loud = await level()
    // The synth's gain down to its minimum: the meter falls (the report is the engine's).
    await page.getByRole('slider', { name: /gain/i }).press('Home')
    await expect.poll(level, { timeout: 5000 }).toBeLessThan(loud - 20)
    await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x80, 69, 0]))
    await expect(page.getByText('Voices: 0')).toBeVisible({ timeout: 5000 })
    expect(errors).toEqual([])
  })
})
