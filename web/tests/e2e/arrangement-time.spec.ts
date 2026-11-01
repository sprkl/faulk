import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #396: insert and delete bars across all tracks, in a real browser. "Insert bars" opens empty bars at the play
// position's bar (or at a time selection), a Shift+drag on the ruler selects time, "Delete selected bars" closes
// it up, and each action is one undo.

const RULER = 28
const BAR = 3840

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const arr = (page: Page) => page.getByTestId('arrangement')
const stage = (page: Page) => page.getByTestId('arr-stage')
const starts = async (page: Page): Promise<number[]> =>
  (JSON.parse((await arr(page).getAttribute('data-blocks')) ?? '[]') as { start: number }[]).map((b) => b.start).sort((a, b) => a - b)

test('arrangement view: insert bars, select time with Shift+drag, delete the selected bars, each one undo', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1800, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()

  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toHaveAttribute('data-block-count', '1')
  expect(await starts(page)).toEqual([0])
  await page.getByTestId('arr-snap').selectOption('bar')

  // Two bars inserted at the play position (bar 1): the clip moves two bars later.
  await page.getByTestId('arr-bars').fill('2')
  await page.getByTestId('arr-insert-bars').click()
  await expect.poll(() => starts(page)).toEqual([2 * BAR])
  await page.keyboard.press('Control+z')
  await expect.poll(() => starts(page)).toEqual([0])
  await page.keyboard.press('Control+Shift+z')
  await expect.poll(() => starts(page)).toEqual([2 * BAR])

  // Select the two empty bars with Shift+drag on the ruler, delete them: the clip is back at the start.
  await expect(page.getByTestId('arr-delete-bars')).toBeDisabled()
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const y = box.y + RULER / 2
  await page.keyboard.down('Shift')
  await page.mouse.move(box.x + 1, y)
  await page.mouse.down()
  await page.mouse.move(box.x + BAR * ppt, y, { steps: 4 })
  await page.mouse.move(box.x + 2 * BAR * ppt, y, { steps: 4 })
  await page.mouse.up()
  await page.keyboard.up('Shift')
  await expect(page.getByTestId('arr-delete-bars')).toBeEnabled()
  await page.getByTestId('arr-delete-bars').click()
  await expect.poll(() => starts(page)).toEqual([0])
  await expect(page.getByTestId('arr-delete-bars')).toBeDisabled()
  await page.keyboard.press('Control+z')
  await expect.poll(() => starts(page)).toEqual([2 * BAR])

  expect(seen.errors).toEqual([])
})
