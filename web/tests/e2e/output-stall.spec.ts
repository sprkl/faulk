import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #810: when the audio output stops pulling the Web engine, the context's clock stands still and the position and the
// meters freeze on their last values. The page says so in a notice under the top bar ("No audio output: playback cannot
// advance") instead of freezing without a word, and clears it once the clock runs again. A stalled output is simulated
// by suspending the context behind the page's back (an init script keeps a handle on it): the page did not ask for it,
// and the clock stops as it does when nothing consumes the audio.

test.describe.configure({ mode: 'serial' })

type Ctx = { __ctx810: AudioContext }

async function start(page: Page) {
  await page.addInitScript(() => {
    const Original = window.AudioContext
    window.AudioContext = class extends Original {
      constructor(options?: AudioContextOptions) {
        super(options)
        ;(window as unknown as Ctx).__ctx810 = this
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

const tick = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-tick'))
const notice = (page: Page) => page.getByTestId('no-output-notice')
const transport = (page: Page, name: string) => page.getByTestId('transport').getByRole('button', { name, exact: true })
const context = (page: Page, action: 'suspend' | 'resume') => page.evaluate((a) => (window as unknown as Ctx).__ctx810[a](), action)

test('playback advances for 3 s with no notice, then a stalled output is said and cleared when it comes back', async ({ page }) => {
  test.setTimeout(120_000)
  const { errors } = watch(page)
  await start(page)
  await transport(page, 'Play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')

  // The position keeps advancing well past the first half second (the freeze of #810 came after ~0.5 s).
  const ticks: number[] = []
  for (let i = 0; i < 7; i++) {
    ticks.push(await tick(page))
    await page.waitForTimeout(500)
  }
  for (let i = 1; i < ticks.length; i++) expect(ticks[i], `position at +${i * 0.5} s: ${ticks.join(', ')}`).toBeGreaterThan(ticks[i - 1]!)
  await expect(notice(page)).toHaveCount(0)

  // The output stops pulling: the position freezes, and the transport says why within a couple of seconds.
  await context(page, 'suspend')
  await expect(notice(page)).toHaveText('No audio output: playback cannot advance', { timeout: 5000 })
  await expect(page.getByTestId('engine-mode')).toHaveAttribute('title', /No audio output: playback cannot advance/)
  const frozen = await tick(page)
  await page.waitForTimeout(500)
  expect(await tick(page)).toBe(frozen)

  // Stop and Play while the output is stalled: the notice stays, the engine cannot take them before the clock runs.
  await transport(page, 'Stop').click()
  await page.waitForTimeout(300)
  await expect(notice(page)).toBeVisible()

  // The output comes back: the notice goes at once, and the engine plays the commands it was given meanwhile.
  await context(page, 'resume')
  await expect(notice(page)).toHaveCount(0, { timeout: 2000 })
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'stopped')
  await transport(page, 'Play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  const resumed = await tick(page)
  await expect.poll(() => tick(page), { timeout: 5000 }).toBeGreaterThan(resumed)
  await page.waitForTimeout(1500)
  await expect(notice(page)).toHaveCount(0)
  expect(errors).toEqual([])
})

test('the notice is in French with the page in French', async ({ page }) => {
  const { errors } = watch(page)
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
  await start(page)
  await page.waitForTimeout(300)
  await context(page, 'suspend')
  // Stopped transport: the output that does not pull is said all the same, since Play could not advance either.
  await expect(notice(page)).toHaveText('Aucune sortie audio : la lecture ne peut pas avancer', { timeout: 5000 })
  await context(page, 'resume')
  await expect(notice(page)).toHaveCount(0, { timeout: 2000 })
  expect(errors).toEqual([])
})

test('a running context whose clock stands still is said too, and cleared when it moves again', async ({ page }) => {
  // The case the issue most likely met: the context says `running`, but nothing pulls it, so its clock stands still. The
  // clock the page reads is frozen on this context only (its getter shadowed on the instance); the engine keeps playing.
  const { errors } = watch(page)
  await start(page)
  await transport(page, 'Play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  await page.evaluate(() => {
    const ctx = (window as unknown as Ctx).__ctx810
    const frozen = ctx.currentTime
    Object.defineProperty(ctx, 'currentTime', { configurable: true, get: () => frozen })
  })
  expect(await page.evaluate(() => (window as unknown as Ctx).__ctx810.state)).toBe('running')
  await expect(notice(page)).toHaveText('No audio output: playback cannot advance', { timeout: 5000 })
  await page.evaluate(() => delete (window as unknown as { __ctx810: { currentTime?: number } }).__ctx810.currentTime)
  await expect(notice(page)).toHaveCount(0, { timeout: 2000 })
  await page.waitForTimeout(1500)
  await expect(notice(page)).toHaveCount(0)
  expect(errors).toEqual([])
})
