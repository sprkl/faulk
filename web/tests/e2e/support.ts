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
