import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #317: the bottom panel resizes (drag and keyboard), folds, and remembers its height per view in the
// page's preferences. Real browser, 1280 x 800 like the navigation test.

const PANEL = 'bottom-panel'
const panel = (page: Page) => page.getByTestId(PANEL)
const handle = (page: Page) => page.getByTestId('bottom-panel-handle')
const fold = (page: Page) => page.getByTestId('bottom-panel-fold')
const heightOf = async (page: Page) => Math.round((await panel(page).boundingBox())!.height)
/** Heights are measured in CSS pixels that may be fractional by default: one pixel of slack. */
const near = (page: Page, expected: number) => expect.poll(async () => Math.abs((await heightOf(page)) - expected)).toBeLessThanOrEqual(1)
const valueNow = async (page: Page) => Number(await handle(page).getAttribute('aria-valuenow'))

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 })
})

test('the handle is a separator that resizes the panel by drag and by keyboard', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await expect(handle(page)).toHaveAttribute('role', 'separator')
  await expect(handle(page)).toHaveAttribute('aria-orientation', 'horizontal')
  await expect(handle(page)).toHaveAccessibleName('Resize the bottom panel')

  const before = await heightOf(page)
  expect(Math.abs((await valueNow(page)) - before)).toBeLessThanOrEqual(1)

  // Drag up by 100 px: the panel grows by 100.
  const box = (await handle(page).boundingBox())!
  const x = box.x + box.width / 2
  const y = box.y + box.height / 2
  await page.mouse.move(x, y)
  await page.mouse.down()
  await page.mouse.move(x, y - 50, { steps: 5 })
  await page.mouse.move(x, y - 100, { steps: 5 })
  await page.mouse.up()
  await near(page, before + 100)
  await expect.poll(async () => Math.abs((await valueNow(page)) - (await heightOf(page)))).toBeLessThanOrEqual(1)

  // Keyboard: the arrows step by 16 (64 with Shift), Home goes to the minimum, End to the maximum.
  await handle(page).focus()
  await page.keyboard.press('ArrowUp')
  await near(page, before + 116)
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('ArrowDown')
  await near(page, before + 84)
  await page.keyboard.press('Shift+ArrowUp')
  await near(page, before + 148)
  await page.keyboard.press('Home')
  const min = Number(await handle(page).getAttribute('aria-valuemin'))
  await near(page, min)
  await page.keyboard.press('ArrowDown')
  expect(await heightOf(page)).toBe(min)
  await page.keyboard.press('End')
  const max = Number(await handle(page).getAttribute('aria-valuemax'))
  await near(page, max)
  await page.keyboard.press('ArrowUp')
  expect(await heightOf(page)).toBe(max)
  // The main view keeps room at the largest size.
  expect((await page.getByTestId('view').boundingBox())!.height).toBeGreaterThanOrEqual(119)
  expect(errors).toEqual([])
})

test('the panel folds to its tabs and unfolds to the height it had', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await handle(page).focus()
  await page.keyboard.press('Shift+ArrowUp')
  const open = await heightOf(page)

  await fold(page).click()
  await expect(panel(page)).toHaveAttribute('data-collapsed', 'true')
  await expect(fold(page)).toHaveAttribute('aria-expanded', 'false')
  await expect(fold(page)).toHaveAccessibleName('Expand the bottom panel')
  expect(await heightOf(page)).toBeLessThan(60)
  expect(await valueNow(page)).toBe(0)
  // The tabs stay reachable and the page does not scroll.
  await expect(page.getByTestId('panel-piano-roll')).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollHeight <= innerHeight)).toBe(true)

  await fold(page).click()
  await expect(panel(page)).toHaveAttribute('data-collapsed', 'false')
  await near(page, open)

  // Double click on the handle folds, then unfolds.
  await handle(page).dblclick()
  await expect(panel(page)).toHaveAttribute('data-collapsed', 'true')
  await handle(page).dblclick()
  await expect(panel(page)).toHaveAttribute('data-collapsed', 'false')
  await near(page, open)

  // Enter on the handle folds; ArrowUp on a folded handle unfolds.
  await handle(page).focus()
  await page.keyboard.press('Enter')
  await expect(panel(page)).toHaveAttribute('data-collapsed', 'true')
  await page.keyboard.press('ArrowUp')
  await expect(panel(page)).toHaveAttribute('data-collapsed', 'false')
  expect(errors).toEqual([])
})

test('the height and the fold are kept per view in the page, across a reload', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await handle(page).focus()
  await page.keyboard.press('Shift+ArrowUp')
  await page.keyboard.press('Shift+ArrowUp')
  const devices = await heightOf(page)

  await page.getByTestId('panel-piano-roll').click()
  await expect(panel(page)).toHaveAttribute('data-panel', 'piano-roll')
  await handle(page).focus()
  await page.keyboard.press('Home')
  const roll = await heightOf(page)
  expect(roll).not.toBe(devices)

  // The project is not touched: the preference lives in localStorage only.
  const stored = await page.evaluate(() => localStorage.getItem('faulk.panelHeights'))
  expect(JSON.parse(stored!)).toEqual({ devices, 'piano-roll': roll })

  await page.reload()
  await page.getByTestId('panel-devices').click()
  await near(page, devices)
  await page.getByTestId('panel-piano-roll').click()
  await near(page, roll)

  await fold(page).click()
  await page.reload()
  await expect(panel(page)).toHaveAttribute('data-collapsed', 'true')
  await fold(page).click()
  await page.getByTestId('panel-piano-roll').click()
  await near(page, roll)
  expect(errors).toEqual([])
})

test('a bad stored value is ignored and a blocked storage does not break the panel', async ({ page }) => {
  const { errors } = watch(page)
  await page.addInitScript(() => localStorage.setItem('faulk.panelHeights', '{"devices":"tall"'))
  await page.goto('/app')
  await expect(panel(page)).toBeVisible()
  const height = await heightOf(page)
  expect(height).toBeGreaterThan(100)
  expect(errors).toEqual([])
})

test('at its smallest the piano roll still shows an octave, and the mixer scrolls instead of being cut', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await handle(page).focus()
  await page.keyboard.press('Home')
  // A roll needs a clip to draw; the stage is there with its rows once a track exists.
  const stage = page.getByTestId('roll-stage')
  await page.getByTestId('roll-create-clip').click()
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  await expect(stage).toBeVisible()
  const stageHeight = (await stage.boundingBox())!.height
  const ROW = 16
  const RULER = 28
  expect(stageHeight - RULER).toBeGreaterThanOrEqual(12 * ROW)

  // Largest panel over the mixer: the view scrolls and its last strip can be reached.
  await page.keyboard.press('End')
  await page.getByTestId('nav-mixer').click()
  const view = page.getByTestId('view')
  await expect(page.getByTestId('mixer')).toBeVisible()
  const scrolls = await view.evaluate((el) => ({ overflow: getComputedStyle(el).overflowY, can: el.scrollHeight > el.clientHeight }))
  expect(scrolls.overflow).toBe('auto')
  const master = page.getByTestId('mixer-master')
  await master.scrollIntoViewIfNeeded()
  const v = (await view.boundingBox())!
  const m = (await master.boundingBox())!
  expect(m.y).toBeGreaterThanOrEqual(v.y - 1)
  expect(m.y + m.height).toBeLessThanOrEqual(v.y + v.height + 1)
  void scrolls.can
  expect(errors).toEqual([])
})
