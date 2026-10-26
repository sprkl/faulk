import type { Page } from '@playwright/test'

/** The helper's address (`src/lib/engine/helper-client.ts`). */
export const HELPER_HOST = '127.0.0.1'
/** `FAULK_E2E_HELPER_PORT` moves the stand-in helper (and the page's target, at build time) when 47800 is held by an installed helper. */
export const HELPER_PORT = Number(process.env.FAULK_E2E_HELPER_PORT) || 47800

/**
 * The page probes the helper on load; with no helper listening, the browser logs the refused
 * WebSocket as a console error. That one is expected and is the only one ignored.
 */
const REFUSED_PROBE = new RegExp(String.raw`^WebSocket connection to 'ws://127\.0\.0\.1:${HELPER_PORT}/probe' failed`)

/** Collects console errors and uncaught exceptions, and every request the page makes. */
export function watch(page: Page) {
  const errors: string[] = []
  const requests: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error' && !REFUSED_PROBE.test(message.text())) errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(String(error)))
  page.on('request', (request) => requests.push(request.url()))
  return { errors, requests }
}

/** The requests whose origin is not the page's (`data:` and `blob:` URLs are the page's own). */
export function foreignRequests(requests: string[], baseURL: string): string[] {
  const origin = new URL(baseURL).origin
  return requests.filter((url) => !/^(data|blob):/.test(url) && new URL(url).origin !== origin)
}

/** A fake Web MIDI access with one input, `window.__fakeMidi.send(bytes)` plays through it. */
export async function fakeWebMidi(page: Page) {
  await page.addInitScript(() => {
    const input = {
      id: 'e2e-keys',
      name: 'E2E Keys',
      state: 'connected',
      onmidimessage: null as ((event: { data: Uint8Array }) => void) | null,
      close: () => Promise.resolve(),
    }
    const inputs = new Map([[input.id, input]])
    const access = { inputs, outputs: new Map(), onstatechange: null }
    Object.defineProperty(navigator, 'requestMIDIAccess', { value: () => Promise.resolve(access) })
    Object.assign(window, { __fakeMidi: { send: (bytes: number[]) => input.onmidimessage?.({ data: new Uint8Array(bytes) }) } })
  })
}

/**
 * Timing budgets of CT-3 (docs/agents/verification.md, "Timing budgets"). A shared CI runner is
 * slower and far noisier than a workstation, and one preempted quantum is enough to push the
 * longest pump past 500 µs (the prototype measured 512 µs in CI; the shipped worklet is far below, see verification.md), so by default a budget is only held by a
 * ceiling ten times higher: it catches a change of order of magnitude, not of a few percent.
 * `FAULK_E2E_STRICT_TIMING=1` holds the strict budget itself, on the median of
 * `STRICT_RUNS` measurements, on a reference machine. Any other non-empty value is an error: it
 * would silently run the ceiling.
 */
const flag = process.env.FAULK_E2E_STRICT_TIMING ?? ''
if (flag !== '' && flag !== '0' && flag !== '1') throw new Error(`FAULK_E2E_STRICT_TIMING must be 1 or 0, got "${flag}"`)
export const STRICT_TIMING = flag === '1'
export const STRICT_RUNS = 5

/** The budget to assert: `strict` on a reference machine, ten times it otherwise. */
export function timingLimit(strict: number): number {
  return STRICT_TIMING ? strict : strict * 10
}

/** Ten times a budget, for a measurement held by the ceiling in every mode (single sample). */
export function ceiling(strict: number): number {
  return strict * 10
}

export function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.floor(sorted.length / 2)]
}

/**
 * Main-thread time the page itself spent, from the browser's counters, rather than wall-clock frame
 * intervals: an interval also counts the time the runner kept the browser waiting (a busy neighbour,
 * the vsync tick), so on a shared machine it moves with the machine and not with the code. The
 * task duration is the time the main thread worked, and a regression in the page raises it.
 */
export interface MainThreadCost {
  /** Total main-thread task time over the window, in ms. */
  task: number
  script: number
  layout: number
  style: number
  /** The time a fixed piece of JavaScript takes on this machine right now, in ms: the unit that makes a cost comparable across runners. */
  unit: number
}

/** A fixed amount of arithmetic, timed in the page: it slows with the machine as the page's own work does. */
function referenceWork(): number {
  const start = performance.now()
  let x = 0
  for (let i = 0; i < 4_000_000; i++) x += Math.sqrt(i) * (i & 3)
  return performance.now() - start + (x < 0 ? 1 : 0)
}

export async function mainThreadMeter(page: Page) {
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Performance.enable')
  // `FAULK_E2E_CPU_THROTTLE=4` rehearses a slow runner (CDP CPU slowdown), for the local proof of a test's stability.
  const throttle = Number(process.env.FAULK_E2E_CPU_THROTTLE) || 1
  if (throttle > 1) await cdp.send('Emulation.setCPUThrottlingRate', { rate: throttle })
  const read = async () => {
    const { metrics } = await cdp.send('Performance.getMetrics')
    const by = Object.fromEntries(metrics.map((m) => [m.name, m.value]))
    return { task: by.TaskDuration!, script: by.ScriptDuration!, layout: by.LayoutDuration!, style: by.RecalcStyleDuration! }
  }
  return {
    /** Runs `run` and returns its result with the main-thread time spent meanwhile. */
    async measure<T>(run: () => Promise<T>): Promise<{ result: T; cost: MainThreadCost }> {
      const unit = () => page.evaluate(referenceWork)
      const unitBefore = await unit()
      const before = await read()
      const result = await run()
      const after = await read()
      const unitAfter = await unit()
      const ms = (key: 'task' | 'script' | 'layout' | 'style') => (after[key] - before[key]) * 1000
      return { result, cost: { task: ms('task'), script: ms('script'), layout: ms('layout'), style: ms('style'), unit: Math.min(unitBefore, unitAfter) } }
    },
  }
}

/** How many times a measurement whose clock drifted is taken again before it is given up (#359). */
export const CLOCK_ATTEMPTS = 4

/**
 * Runs a measurement and takes it again when its clock drifted. The counter the timings are read
 * from is a Worker spinning on a shared counter, and a shared runner changes its pace while the
 * other threads load the same core (a two-vCPU runner flips between about 55 and 110 ticks/us
 * within one run, in CI run 40388605597): every figure of that run is then wrong by as much.
 * `drift` returns a description of the drift, or nothing when the clock held.
 *
 * Strict mode (reference machine): up to CLOCK_ATTEMPTS runs, then it fails by name; the budgets
 * are never touched. Otherwise: two runs, then the last one is kept with a printed note, because
 * the only assertions are the ceilings at ten times the budget, and a clock that is off by 2x
 * cannot hide a gross regression from them (an injected busy loop still fails them). A real
 * regression is not a drift: it fails the assertions of the run that is kept.
 */
export async function withStableClock<T>(what: string, run: () => Promise<T>, drift: (result: T) => string | undefined): Promise<T> {
  const attempts = STRICT_TIMING ? CLOCK_ATTEMPTS : 2
  const seen: string[] = []
  let result: T
  for (let attempt = 1; attempt <= attempts; attempt++) {
    result = await run()
    const problem = drift(result)
    if (!problem) return result
    seen.push(problem)
    console.log(`${what}: attempt ${attempt}/${attempts} dropped, the clock drifted (${problem})`)
  }
  if (STRICT_TIMING) throw new Error(`${what}: the clock drifted in all ${attempts} attempts, nothing measured (${seen.join('; ')}); the machine is not a reference machine, not a regression of the code`)
  console.log(`${what}: the clock drifted in both attempts, the last run is kept and held to the ceilings only (10x the budgets), not to the budgets`)
  return result!
}

/** The smallest of the values: noise on a shared runner only ever adds time, so the best window is the truest. */
export function best(values: number[]): number {
  return Math.min(...values)
}

/** One line for the test output: which limits were applied. */
export function timingMode(strict: number[]): string {
  return STRICT_TIMING
    ? `timing: STRICT, median of ${STRICT_RUNS} runs against ${strict.join(' / ')}`
    : `timing: ceiling (10x) ${strict.map((n) => n * 10).join(' / ')}; strict budgets ${strict.join(' / ')} with FAULK_E2E_STRICT_TIMING=1`
}
