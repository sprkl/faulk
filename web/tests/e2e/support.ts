import type { Page } from '@playwright/test'

/** The helper's address (`src/lib/engine/helper-client.ts`). */
export const HELPER_HOST = '127.0.0.1'
export const HELPER_PORT = 47800

/**
 * The page probes the helper on load; with no helper listening, the browser logs the refused
 * WebSocket as a console error. That one is expected and is the only one ignored.
 */
const REFUSED_PROBE = /^WebSocket connection to 'ws:\/\/127\.0\.0\.1:47800\/probe' failed/

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

/** One line for the test output: which limits were applied. */
export function timingMode(strict: number[]): string {
  return STRICT_TIMING
    ? `timing: STRICT, median of ${STRICT_RUNS} runs against ${strict.join(' / ')}`
    : `timing: ceiling (10x) ${strict.map((n) => n * 10).join(' / ')}; strict budgets ${strict.join(' / ')} with FAULK_E2E_STRICT_TIMING=1`
}
