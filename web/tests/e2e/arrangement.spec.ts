import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// AR-2 (#78): the engine plays the arrangement. A clip drawn in the piano roll sits in the arrangement
// (a placement from tick 0): pressing Play in the transport bar makes it sound through the real worklet
// and engine, Stop silences it, and `launch.back_to_arrangement` reaches the engine without an error.

const SILENCE_DB = -60
const ROW = 16
const RULER = 28

const startAudio = (page: Page) => page.getByTestId('engine-state').getByRole('button', { name: /Start audio|Démarrer/ }).click()
const meterDb = async (page: Page) => Number(await page.getByRole('meter').getAttribute('aria-valuenow'))
const transport = (page: Page, name: string) => page.getByTestId('transport').getByRole('button', { name, exact: true })
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)

async function spot(page: Page) {
  const stage = page.getByTestId('roll-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await page.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await stage.getAttribute('data-scroll-y'))
  return {
    x: (tick: number) => box.x + tick * ppt,
    y: (pitch: number) => box.y + RULER + (127 - pitch) * ROW - scrollY + ROW / 2,
  }
}

test('arrangement: a clip drawn in the piano roll sounds on Play, is silent on Stop and the grid hands the track back', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await page.getByTestId('roll-snap').selectOption('beat')

  // Three notes of one beat, from the start of the clip: a click draws a note (and plays it while pressed).
  const g = await spot(page)
  for (const tick of [10, 1000, 1970]) {
    await page.mouse.click(g.x(tick), g.y(60 + (tick > 1500 ? 4 : 0)))
  }
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '3')
  // Nothing plays before Play: the audition is over.
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBe(SILENCE_DB)

  // Play: the arrangement sounds, with no note played by hand.
  await transport(page, 'Play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)

  // Stop: the notes end, the meter falls back to silence.
  await transport(page, 'Stop').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'stopped')
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBe(SILENCE_DB)

  // Play again, and give the tracks back to the arrangement while it plays: the command reaches the
  // engine, the arrangement goes on.
  await transport(page, 'Play').click()
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  // The dispatcher's dev hook exists in the dev server only (the production build has none).
  if ((await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) === 'function') {
    await run(page, { type: 'launch.back_to_arrangement' })
  }
  // Back to the start while it plays: the note that starts on tick 0 starts again, on the first sample.
  await transport(page, 'Back to start').click()
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await transport(page, 'Stop').click()
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBe(SILENCE_DB)

  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/arrangement.png` })
  expect(seen.errors).toEqual([])
})
