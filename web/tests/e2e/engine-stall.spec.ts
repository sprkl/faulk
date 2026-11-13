import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #877: when the Web engine stops responding while the audio output still pulls it (its render side no longer runs, so
// the report's `seq` stands still while the context's clock advances), the page says "The engine stopped responding"
// in the notice line under the top bar, with a Restart button, instead of freezing without a word. It clears once
// reports come again. A stuck engine is simulated without touching the app: an init script loads a test module into
// the audio worklet before the app's, which wraps `registerProcessor` so that the engine's processor skips its
// `process` (renders nothing, writes no report, the clock runs on) while a flag in a shared buffer is set. Each node
// the page builds gets its own flag (`__stuck877`, the last node's), so a restarted engine starts unstuck.

test.describe.configure({ mode: 'serial' })

type Win = { __stuck877: Int32Array; __ctx877: AudioContext }

async function start(page: Page, lang?: 'fr') {
  if (lang) await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.addInitScript(() => {
    const win = window as unknown as Win
    const stub = `
      const original = globalThis.registerProcessor
      globalThis.registerProcessor = (name, Processor) =>
        original(name, class extends Processor {
          constructor(options) {
            super(options)
            const flag = options && options.processorOptions && options.processorOptions.stuck877
            this.stuck877 = flag ? new Int32Array(flag) : null
          }
          process(inputs, outputs, parameters) {
            return this.stuck877 && Atomics.load(this.stuck877, 0) === 1 ? true : super.process(inputs, outputs, parameters)
          }
        })
    `
    const stubUrl = URL.createObjectURL(new Blob([stub], { type: 'text/javascript' }))
    const addModule = AudioWorklet.prototype.addModule
    const stubbed = new WeakSet<AudioWorklet>()
    AudioWorklet.prototype.addModule = async function (this: AudioWorklet, url: string | URL, options?: WorkletOptions) {
      if (!stubbed.has(this)) {
        stubbed.add(this)
        await addModule.call(this, stubUrl)
      }
      return addModule.call(this, url, options)
    }
    const Node = window.AudioWorkletNode
    window.AudioWorkletNode = class extends Node {
      constructor(context: BaseAudioContext, name: string, options: AudioWorkletNodeOptions = {}) {
        const buffer = new SharedArrayBuffer(4)
        super(context, name, { ...options, processorOptions: { ...(options.processorOptions as object | undefined), stuck877: buffer } })
        win.__stuck877 = new Int32Array(buffer)
      }
    }
    const Context = window.AudioContext
    window.AudioContext = class extends Context {
      constructor(options?: AudioContextOptions) {
        super(options)
        win.__ctx877 = this
      }
    }
  })
  await page.goto('/app')
  await page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer l’audio/ })
    .click()
  await engineUp(page, 'Web')
}

const stick = (page: Page, stuck: boolean) => page.evaluate((s) => Atomics.store((window as unknown as Win).__stuck877, 0, s ? 1 : 0), stuck)
const tick = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-tick'))
const notice = (page: Page) => page.getByTestId('engine-stall-notice')
const transport = (page: Page, name: string) => page.getByTestId('transport').getByRole('button', { name, exact: true })

async function play(page: Page) {
  await transport(page, 'Play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  const from = await tick(page)
  await expect.poll(() => tick(page), { timeout: 5000 }).toBeGreaterThan(from)
}

test('a stuck engine is said with a Restart, and the notice goes when the engine reports again', async ({ page }) => {
  test.setTimeout(90_000)
  const { errors } = watch(page)
  await start(page)
  await play(page)
  await page.waitForTimeout(1500)
  await expect(notice(page)).toHaveCount(0)

  await stick(page, true)
  await expect(notice(page)).toContainText('The engine stopped responding', { timeout: 5000 })
  await expect(notice(page).getByRole('button', { name: 'Restart' })).toBeVisible()
  await expect(page.getByTestId('engine-mode')).toHaveAttribute('title', /The engine stopped responding/)
  // The clock runs, the engine does not: the position is frozen, and the output's notice is not the one shown.
  const frozen = await tick(page)
  await page.waitForTimeout(500)
  expect(await tick(page)).toBe(frozen)
  await expect(page.getByTestId('no-output-notice')).toHaveCount(0)

  // The engine renders again: the notice goes at once, and the position moves on.
  await stick(page, false)
  await expect(notice(page)).toHaveCount(0, { timeout: 2000 })
  await expect.poll(() => tick(page), { timeout: 5000 }).toBeGreaterThan(frozen)
  await page.waitForTimeout(1500)
  await expect(notice(page)).toHaveCount(0)
  expect(errors).toEqual([])
})

test('Restart brings a new engine up from the project, which plays', async ({ page }) => {
  test.setTimeout(90_000)
  const { errors } = watch(page)
  await start(page)
  await play(page)
  await stick(page, true)
  await expect(notice(page)).toBeVisible({ timeout: 5000 })
  await notice(page).getByRole('button', { name: 'Restart' }).click()
  await engineUp(page, 'Web')
  await expect(notice(page)).toHaveCount(0, { timeout: 5000 })
  // A new context and a new node (the stub's flag of the new one is clear): the new engine plays.
  expect(await page.evaluate(() => Atomics.load((window as unknown as Win).__stuck877, 0))).toBe(0)
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'stopped', { timeout: 5000 })
  await play(page)
  await page.waitForTimeout(1500)
  await expect(notice(page)).toHaveCount(0)
  await expect(page.getByTestId('stream-error')).toHaveCount(0)
  expect(errors).toEqual([])
})

test('the notice and its button are in French with the page in French', async ({ page }) => {
  const { errors } = watch(page)
  await start(page, 'fr')
  await page.waitForTimeout(300)
  // Transport stopped: the engine still writes its report, so a stuck one is said all the same.
  await stick(page, true)
  await expect(notice(page)).toContainText('Le moteur ne répond plus', { timeout: 5000 })
  await expect(notice(page).getByRole('button', { name: 'Relancer' })).toBeVisible()
  await stick(page, false)
  await expect(notice(page)).toHaveCount(0, { timeout: 2000 })
  expect(errors).toEqual([])
})

test('an output that stalls too wins: only "No audio output" is said, and the engine notice comes back once the clock runs', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await play(page)
  await stick(page, true)
  await expect(notice(page)).toBeVisible({ timeout: 5000 })
  // The output stops pulling (the context suspended behind the page's back, as in #876's spec).
  await page.evaluate(() => (window as unknown as Win).__ctx877.suspend())
  await expect(page.getByTestId('no-output-notice')).toBeVisible({ timeout: 5000 })
  await expect(notice(page)).toHaveCount(0)
  await expect(page.getByTestId('engine-mode')).not.toHaveAttribute('title', /The engine stopped responding/)
  // The output comes back, the engine is still stuck: its word is the one said again.
  await page.evaluate(() => (window as unknown as Win).__ctx877.resume())
  await expect(page.getByTestId('no-output-notice')).toHaveCount(0, { timeout: 2000 })
  await expect(notice(page)).toBeVisible()
  await stick(page, false)
  await expect(notice(page)).toHaveCount(0, { timeout: 2000 })
  expect(errors).toEqual([])
})
