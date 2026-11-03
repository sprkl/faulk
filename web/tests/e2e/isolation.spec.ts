import { expect, test, type Page, type Response } from '@playwright/test'
import { chooseMidiInput, engineUp, fakeWebMidi, foreignRequests, watch } from './support'

// AU-0: the whole site is cross-origin isolated (COOP same-origin + COEP require-corp from
// vercel.json), so that the DAW has SharedArrayBuffer and shared wasm memory for the engine channel
// and audio streaming, and so that any worker script it starts carries COEP too. The landing page is
// isolated as well and must keep working: fonts, animation, no third-party resource.

/** Collects the responses of the page's own origin. */
function ownResponses(page: Page, baseURL: string): Response[] {
  const origin = new URL(baseURL).origin
  const responses: Response[] = []
  page.on('response', (response) => {
    if (new URL(response.url()).origin === origin) responses.push(response)
  })
  return responses
}

/** Every response of the page's origin carries both headers (a worker script needs COEP itself). */
function expectIsolationHeaders(responses: Response[]) {
  expect(responses.length).toBeGreaterThan(0)
  const missing = responses
    .filter((r) => {
      const h = r.headers()
      return h['cross-origin-opener-policy'] !== 'same-origin' || h['cross-origin-embedder-policy'] !== 'require-corp'
    })
    .map((r) => r.url())
  expect(missing).toEqual([])
}

test.describe('the DAW (/app)', () => {
  test('is cross-origin isolated and can share memory', async ({ page, baseURL }) => {
    const { errors, requests } = watch(page)
    const responses = ownResponses(page, baseURL!)
    await page.goto('/app')
    await page.waitForLoadState('networkidle')

    const shared = await page.evaluate(async () => {
      const memory = new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true })
      // Posting a SharedArrayBuffer throws a DataCloneError on a page that is not isolated.
      const channel = new MessageChannel()
      const received = new Promise<boolean>((resolve) => {
        channel.port2.onmessage = (event) => resolve(event.data instanceof SharedArrayBuffer)
      })
      channel.port1.postMessage(memory.buffer)
      return {
        isolated: crossOriginIsolated,
        sharedMemory: memory.buffer instanceof SharedArrayBuffer,
        posted: await received,
      }
    })
    expect(shared).toEqual({ isolated: true, sharedMemory: true, posted: true })
    expectIsolationHeaders(responses)
    expect(foreignRequests(requests, baseURL!)).toEqual([])
    expect(errors).toEqual([])
  })

  test('shares memory with an AudioWorklet (the engine thread)', async ({ page }) => {
    await page.goto('/app')
    // A click gives the page the user activation an AudioContext needs to run its render thread.
    await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
    await engineUp(page)
    const value = await page.evaluate(async () => {
      const processor = `registerProcessor('e2e-shared', class extends AudioWorkletProcessor {
        constructor() {
          super()
          this.port.onmessage = (event) => {
            Atomics.store(new Int32Array(event.data), 0, 42)
            this.port.postMessage(event.data instanceof SharedArrayBuffer)
          }
        }
        process() { return true }
      })`
      const context = new AudioContext()
      await context.resume()
      await context.audioWorklet.addModule(URL.createObjectURL(new Blob([processor], { type: 'text/javascript' })))
      const node = new AudioWorkletNode(context, 'e2e-shared')
      const memory = new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true })
      const shared = await new Promise((resolve) => {
        node.port.onmessage = (event) => resolve(event.data)
        node.port.postMessage(memory.buffer)
        setTimeout(() => resolve('no answer from the worklet'), 5000)
      })
      await context.close()
      return { shared, value: Atomics.load(new Int32Array(memory.buffer), 0) }
    })
    expect(value).toEqual({ shared: true, value: 42 })
  })

  test('starts a dedicated worker served by the site, and shares memory with it', async ({ page, baseURL }) => {
    // A dedicated worker of an isolated page loads only if its script's own response carries COEP.
    // The script is the body of a real response of the server (its headers kept as served).
    await page.route('**/e2e-worker.js', async (route) => {
      const response = await route.fetch({ url: new URL('/favicon.svg', baseURL).href })
      await route.fulfill({
        response,
        contentType: 'text/javascript',
        body: 'onmessage = (e) => { Atomics.store(new Int32Array(e.data), 0, 7); postMessage(crossOriginIsolated) }',
      })
    })
    await page.goto('/app')
    const result = await page.evaluate(
      () =>
        new Promise((resolve) => {
          const worker = new Worker('/e2e-worker.js')
          const memory = new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true })
          worker.onmessage = (event) => resolve({ isolated: event.data, value: Atomics.load(new Int32Array(memory.buffer), 0) })
          worker.onerror = () => resolve('worker refused')
          worker.postMessage(memory.buffer)
        }),
    )
    expect(result).toEqual({ isolated: true, value: 7 })
  })

  test('starts a worker from a script the dev or preview server really serves', async ({ page }) => {
    // The landing animation module is a real script of the build (a hashed chunk in the preview,
    // a module through Vite's transform pipeline in dev): loaded as a module worker, it shows that
    // the server's own script responses carry what a worker of an isolated page needs.
    const scripts: string[] = []
    page.on('request', (r) => /\/landing\/anim(\.[0-9a-f]+\.js|\/index\.ts)$/.test(new URL(r.url()).pathname) && scripts.push(r.url()))
    await page.goto('/')
    const stages = page.locator('[data-fk-anim]')
    await stages.first().scrollIntoViewIfNeeded()
    await expect.poll(() => scripts.length).toBeGreaterThan(0)

    const responses: Record<string, string | undefined> = {}
    page.on('response', (r) => {
      if (r.url() === scripts[0]) responses.coep = r.headers()['cross-origin-embedder-policy']
    })
    await page.goto('/app')
    const result = await page.evaluate(
      (url) =>
        new Promise((resolve) => {
          const worker = new Worker(url, { type: 'module' })
          worker.onerror = (event) => resolve(`worker error: ${event.message || 'refused'}`)
          setTimeout(() => resolve('loaded'), 2000)
        }),
      scripts[0],
    )
    expect(result).toBe('loaded')
    expect(responses.coep).toBe('require-corp')
  })

  test('keeps its query and fragment isolated (pairing links)', async ({ page }) => {
    await page.goto('/app?from=tray#pair=garbage')
    expect(await page.evaluate(() => crossOriginIsolated)).toBe(true)
  })

  test('plays the web engine from simulated Web MIDI under isolation', async ({ page, baseURL }) => {
    await fakeWebMidi(page)
    const { errors, requests } = watch(page)
    await page.goto('/app')
    expect(await page.evaluate(() => crossOriginIsolated)).toBe(true)
    await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
    await engineUp(page, 'Web')
    await chooseMidiInput(page, 'E2E Keys')
    const send = (bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)
    await send([0x90, 60, 100])
    // The voice count comes back from the engine in the AudioWorklet.
    await expect(page.getByText('Voices: 1')).toBeVisible()
    await send([0x80, 60, 0])
    await expect(page.getByText('Voices: 0')).toBeVisible({ timeout: 5000 })
    expect(foreignRequests(requests, baseURL!)).toEqual([])
    expect(errors).toEqual([])
  })
})

test.describe('the landing page', () => {
  for (const path of ['/', '/en', '/fr']) {
    test(`${path} works isolated: own origin only, fonts, animation`, async ({ page, baseURL }) => {
      const { errors, requests } = watch(page)
      const responses = ownResponses(page, baseURL!)
      await page.goto(path)
      expect(await page.evaluate(() => crossOriginIsolated)).toBe(true)

      // Bring every sound object near the viewport so the animation loads and starts on each.
      const stages = page.locator('[data-fk-anim]')
      const count = await stages.count()
      expect(count).toBeGreaterThan(0)
      for (let i = 0; i < count; i++) await stages.nth(i).scrollIntoViewIfNeeded()
      await expect(page.locator('.is-live')).toHaveCount(count)
      await page.evaluate(() => document.fonts.ready)
      const fonts = await page.evaluate(() => [...document.fonts].filter((f) => f.status === 'loaded').map((f) => f.family.replace(/"/g, '')))
      expect(fonts.some((f) => f.startsWith('Archivo'))).toBe(true)
      expect(fonts.some((f) => f.startsWith('Geist'))).toBe(true)

      expectIsolationHeaders(responses)
      expect(foreignRequests(requests, baseURL!)).toEqual([])
      expect(errors).toEqual([])
    })
  }
})
