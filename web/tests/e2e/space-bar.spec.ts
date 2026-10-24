import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #332: Space plays and pauses from everywhere. A control the mouse clicked keeps the focus but not Space;
// a control the keyboard focused, and every text field, keep it. Real worklet and engine.

const feedback = (page: Page) => page.getByTestId('feedback')
const tick = async (page: Page) => Number(await feedback(page).getAttribute('data-tick'))
const transport = (page: Page, state: 'playing' | 'stopped') => expect(feedback(page)).toHaveAttribute('data-transport', state)

async function start(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
}

/** Space plays, Space pauses where it is, Space resumes from there. */
async function playPauseResume(page: Page) {
  await page.keyboard.press('Space')
  await transport(page, 'playing')
  await expect.poll(() => tick(page)).toBeGreaterThan(2500)
  await page.keyboard.press('Space')
  await transport(page, 'stopped')
  // The pause is a stop and a seek to the playhead: give the seek time to show before reading the position.
  await page.waitForTimeout(300)
  const paused = await tick(page)
  expect(paused).toBeGreaterThan(2500)
  await page.waitForTimeout(300)
  expect(await tick(page)).toBe(paused)
  await page.keyboard.press('Space')
  await transport(page, 'playing')
  await page.waitForTimeout(400)
  expect(await tick(page)).toBeGreaterThanOrEqual(paused)
  await page.keyboard.press('Space')
  await transport(page, 'stopped')
}

for (const view of ['arrangement', 'grid', 'mixer', 'play']) {
  test(`after a mouse click on the ${view} tab, Space plays and pauses`, async ({ page }) => {
    const { errors } = watch(page)
    await start(page)
    await page.getByTestId(`nav-${view}`).click()
    await expect(page.getByTestId('view')).toHaveAttribute('data-view', view)
    await playPauseResume(page)
    expect(errors).toEqual([])
  })
}

test('after a mouse click on an arrangement track header, Space plays and pauses', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('nav-arrangement').click()
  await page.getByTestId('arr-track').first().click()
  await expect(page.getByTestId('arr-track').first()).toBeFocused()
  await playPauseResume(page)
  expect(errors).toEqual([])
})

test('a control the keyboard focused keeps Space, and so does a text field', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('nav-arrangement').click()
  // Tab moves the focus to the next tab: Space presses it (the view changes) and does not play.
  await page.keyboard.press('Tab')
  await expect(page.getByTestId('nav-grid')).toBeFocused()
  await page.keyboard.press('Space')
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'grid')
  await page.waitForTimeout(400)
  await transport(page, 'stopped')
  // The tempo field takes the space bar even after a mouse click.
  await page.getByTestId('transport-tempo').click()
  await page.keyboard.press('Space')
  await page.waitForTimeout(400)
  await transport(page, 'stopped')
  expect(errors).toEqual([])
})
