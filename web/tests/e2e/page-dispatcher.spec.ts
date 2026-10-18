import { expect, test, type Page } from '@playwright/test'
import { fakeWebMidi, watch } from './support'

// PR-5: the page owns one project core; every command goes through it, the engine gets the bytes
// it encoded, the project store is fed by its events. Real worklet, real engine, real core.

const startAudio = (page: Page) => page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
const meterDb = async (page: Page) => Number(await page.getByRole('meter').getAttribute('aria-valuenow'))
const SILENCE_DB = -60

test('a played key makes sound and a synth change reaches the project store', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  expect(await meterDb(page)).toBe(SILENCE_DB)

  // Synth parameter: the choice the view shows comes from the project store, fed by the core's events.
  const square = page.getByRole('radio', { name: 'Square' })
  await square.click()
  await expect(square).toHaveAttribute('aria-checked', 'true')
  const knob = page.locator('input[type=range]').first()
  const before = await knob.getAttribute('aria-valuetext')
  await knob.fill('0.9')
  await expect.poll(() => knob.getAttribute('aria-valuetext')).not.toBe(before)

  // A key: the engine renders it, the meter reads a non-zero level.
  await page.keyboard.down('KeyA')
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await page.keyboard.up('KeyA')

  expect(errors).toEqual([])
})

test('a Web MIDI note plays through the same commands', async ({ page }) => {
  const { errors } = watch(page)
  await fakeWebMidi(page)
  await page.goto('/app')
  await startAudio(page)
  await page.getByRole('radio', { name: 'E2E Keys' }).check()
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x90, 60, 100]))
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x80, 60, 0]))
  expect(errors).toEqual([])
})

test('a stream the engine refuses stops audio with a message, and restarting keeps the project', async ({ page }) => {
  const { errors } = watch(page)
  // Corrupts the next stream the page hands to the engine's prepare Worker: a real refusal by the engine.
  await page.addInitScript(() => {
    const post = Worker.prototype.postMessage
    Worker.prototype.postMessage = function (this: Worker, message: unknown, ...rest: unknown[]) {
      const flag = (window as unknown as { __corrupt?: boolean }).__corrupt
      if (flag && (message as { type?: string })?.type === 'push') {
        ;(window as unknown as { __corrupt?: boolean }).__corrupt = false
        message = { type: 'push', bytes: new Uint8Array(64).fill(0xee).buffer }
        rest = []
      }
      return (post as (...a: unknown[]) => void).call(this, message, ...rest)
    } as typeof Worker.prototype.postMessage
  })
  await page.goto('/app')
  await startAudio(page)
  await page.getByRole('radio', { name: 'Square' }).click()
  await expect(page.getByRole('radio', { name: 'Square' })).toHaveAttribute('aria-checked', 'true')
  expect(errors).toEqual([])

  await page.evaluate(() => ((window as unknown as { __corrupt: boolean }).__corrupt = true))
  await page.getByRole('radio', { name: 'Sine' }).click()
  const message = page.getByTestId('stream-error')
  await expect(message).toHaveText('The audio engine stopped taking changes. Restart audio to continue.')
  await expect(page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' })).toBeVisible()

  // Restart: a new engine starts from a snapshot of the project, which holds the change the engine
  // missed (Sine), and it plays.
  await startAudio(page)
  await expect(message).toHaveCount(0)
  await expect(page.getByRole('radio', { name: 'Sine' })).toHaveAttribute('aria-checked', 'true')
  await page.keyboard.down('KeyA')
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await page.keyboard.up('KeyA')
  // The refusal is logged once by the page (the engine's own report), nothing else.
  expect(errors.filter((e) => !/^engine refused a stream/.test(e))).toEqual([])
})
