import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #475 (epic #468): the quick wins of the design review, in a real browser at 1920x1080: the app opens
// on the arrangement (then on the view used last), a view keeps its zoom and scroll across a switch, and a
// track's colour is chosen from the sixteen hues of the palette with the keyboard or the pointer.

// A first visit: the other specs start as if the Instrument view was used last (`playwright.config.ts`).
test.use({ storageState: { cookies: [], origins: [] } })

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()

test('opens on the arrangement, then on the view used last; the arrangement keeps its zoom across a switch', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1920, height: 1080 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'arrangement')

  const grid = page.getByTestId('arr-stage').getByTestId('timegrid')
  const before = Number(await grid.getAttribute('data-px-per-tick'))
  await page.getByTestId('arr-zoom-in').click()
  await expect.poll(async () => Number(await grid.getAttribute('data-px-per-tick'))).toBeGreaterThan(before)
  const zoomed = Number(await grid.getAttribute('data-px-per-tick'))

  await page.getByTestId('nav-mixer').click()
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'mixer')
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('arr-stage').getByTestId('timegrid')).toHaveAttribute('data-px-per-tick', String(zoomed))

  // The next visit opens on the view used last.
  await page.getByTestId('nav-mixer').click()
  await page.reload()
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'mixer')
  expect(seen.errors).toEqual([])
})

test('the colour of a track is chosen from the palette, with the keyboard or the pointer; the focus shows on the panel handle', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1920, height: 1080 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: 'Oscillator', exact: true }).click()
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: 'Oscillator', exact: true }).click()

  const chips = page.locator('[data-testid=track-header] .chip')
  const colours = async () => chips.evaluateAll((els) => els.map((el) => getComputedStyle(el).backgroundColor).slice(0, 2))
  // New tracks take the first hues of the palette, none of them the grey of the secondary texts.
  await expect.poll(colours).toEqual(['rgb(240, 128, 152)', 'rgb(241, 138, 118)'])

  const header = page.getByTestId('track-header').first()
  const trigger = header.getByTestId('track-color')
  await trigger.click()
  const hues = page.getByTestId('track-hue')
  await expect(hues).toHaveCount(16)
  for (const box of await hues.evaluateAll((els) => els.map((el) => el.getBoundingClientRect().toJSON()))) {
    expect(box.width).toBeGreaterThanOrEqual(24)
    expect(box.height).toBeGreaterThanOrEqual(24)
  }
  // The names are announced, and the first swatch holds the focus (the track's colour).
  await expect(hues.first()).toBeFocused()
  await expect(hues.first()).toHaveAttribute('aria-label', 'Pink')
  await expect(hues.first()).toHaveAttribute('aria-checked', 'true')
  // Keyboard: right, down (a row of four) and Enter picks the sixth hue.
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('track-palette')).toHaveCount(0)
  await expect(trigger).toBeFocused()
  await expect.poll(colours).toEqual(['rgb(160, 183, 81)', 'rgb(241, 138, 118)'])

  // Escape closes without a change; the pointer picks too.
  await trigger.click()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('track-palette')).toHaveCount(0)
  await trigger.click()
  await hues.nth(15).click()
  await expect.poll(colours).toEqual(['rgb(225, 137, 197)', 'rgb(241, 138, 118)'])
  // One undo puts the previous colour back.
  await page.keyboard.press('Control+z')
  await expect.poll(colours).toEqual(['rgb(160, 183, 81)', 'rgb(241, 138, 118)'])

  // The value button says which value it is.
  await expect(header.getByTestId('track-gain-readout')).toHaveAttribute('aria-label', /^Volume of .+, .+ dB$/)
  // The Snap select is named "Snap", not by its options.
  await expect(page.getByTestId('arr-snap')).toHaveAttribute('aria-label', 'Snap')

  // The name in the arrangement's lane header is the user's text: normal case, the text colour.
  const laneName = page.locator('[data-testid=arr-track] .track-name').first()
  expect(await laneName.evaluate((el) => getComputedStyle(el).textTransform)).toBe('none')
  expect(await laneName.evaluate((el) => getComputedStyle(el).color)).toBe(await page.evaluate(() => getComputedStyle(document.body).color))

  // The Automate pills are 11 px.
  const pill = page.getByTestId('device-param-automate').first()
  await expect(pill).toBeVisible()
  expect(await pill.evaluate((el) => getComputedStyle(el).fontSize)).toBe('11px')

  // The separator of the bottom panel shows its focus ring.
  const handle = page.getByTestId('bottom-panel-handle')
  await handle.focus()
  await page.keyboard.press('Tab')
  await page.keyboard.press('Shift+Tab')
  await expect(handle).toBeFocused()
  expect(await handle.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('solid')
  expect(seen.errors).toEqual([])
})
