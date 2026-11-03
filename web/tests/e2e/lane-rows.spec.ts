import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #471: one row per track in the arrangement (the `lane` token, a height per track), the inspector of the
// selected track in the left column, the volume typed on the selected row, the zoom on Ctrl, and the skip link.
// Real worklet, engine and project core: every edit is a command, read back from the project.

test.describe.configure({ mode: 'serial' })

const SHOTS = '../.qa/design-review-2026-11-01/after-471'

async function start(page: Page, tracks = 1) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('track-header')).toHaveCount(1)
  for (let i = 1; i < tracks; i++) {
    await page.getByTestId('add-track').click()
    await page
      .getByTestId('add-track-menu')
      .getByRole('menuitem')
      .nth(i % 2)
      .click()
  }
  await expect(page.getByTestId('track-header')).toHaveCount(tracks)
}

test('16 tracks fit at 1920x1080 with the bottom panel at 220 px, each row at the lane token', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1920, height: 1080 })
  await page.addInitScript(() => localStorage.setItem('faulk.panelHeights', JSON.stringify({ devices: 220 })))
  await start(page, 16)
  await expect(page.getByTestId('bottom-panel')).toHaveJSProperty('offsetHeight', 220)
  const fit = await page.evaluate(() => {
    const stage = document.querySelector('[data-testid=arr-stage]')!.getBoundingClientRect()
    const rows = [...document.querySelectorAll('[data-testid=track-header]')].map((r) => r.getBoundingClientRect())
    return {
      heights: [...new Set(rows.map((r) => Math.round(r.height)))],
      inside: rows.filter((r) => r.top >= stage.top && r.bottom <= stage.bottom + 0.5).length,
      master: (() => {
        const m = document.querySelector('[data-testid=arr-master-row]')!.getBoundingClientRect()
        return m.top >= stage.top && m.bottom <= stage.bottom + 0.5
      })(),
      token: getComputedStyle(document.documentElement).getPropertyValue('--fk-lane').trim(),
    }
  })
  expect(fit.token).toBe('44px')
  expect(fit.heights).toEqual([44])
  expect(fit.inside).toBe(16)
  // The master row is in sight too.
  expect(fit.master).toBe(true)
  // The rows and the clips' rows line up: the canvas draws a line under each row where the header's border is.
  const heads = page.getByTestId('arr-heads')
  await expect(heads.getByTestId('track-name')).toHaveCount(16)
  mkdirSync(SHOTS, { recursive: true })
  await page.screenshot({ path: `${SHOTS}/arrangement-16-tracks-1920x1080.png` })
  expect(errors).toEqual([])
})

test('a row is resized by its handle, from the pointer and the keyboard, and the clips follow', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page, 2)
  const first = page.getByTestId('track-header').first()
  const handle = first.getByTestId('lane-resize')
  await expect(handle).toHaveAttribute('aria-valuenow', '44')
  await expect(handle).toHaveAttribute('role', 'separator')
  // The pointer: 40 px down.
  const box = (await handle.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 40, { steps: 4 })
  await page.mouse.up()
  await expect(first).toHaveAttribute('data-height', '84')
  // The second row starts where the first ends.
  const [a, b] = await page.getByTestId('track-header').evaluateAll((rows) => rows.map((r) => r.getBoundingClientRect()))
  expect(Math.round(b!.top)).toBe(Math.round(a!.bottom))
  // The keyboard: Up shrinks by 4, Shift+Up by 16, Home to the compact height, End to the highest, Enter back to the token.
  await handle.focus()
  await page.keyboard.press('ArrowUp')
  await expect(handle).toHaveAttribute('aria-valuenow', '80')
  await page.keyboard.press('Shift+ArrowUp')
  await expect(handle).toHaveAttribute('aria-valuenow', '64')
  await page.keyboard.press('Home')
  await expect(handle).toHaveAttribute('aria-valuenow', '28')
  // At the compact height the kind leaves the row (it stays in the head's description).
  await expect(first.locator('.kind')).toHaveCount(0)
  await expect(first.getByTestId('arr-track')).toHaveAccessibleDescription(/MIDI · Oscillator/)
  await page.keyboard.press('End')
  await expect(handle).toHaveAttribute('aria-valuenow', '176')
  await page.keyboard.press('Enter')
  await expect(handle).toHaveAttribute('aria-valuenow', '44')
  // A double click on a resized row's handle gives the default back too.
  await handle.focus()
  await page.keyboard.press('ArrowDown')
  await expect(handle).toHaveAttribute('aria-valuenow', '48')
  await handle.dblclick()
  await expect(handle).toHaveAttribute('aria-valuenow', '44')
  // The compact density takes the lane-compact token, 28 px, for the rows without a height of their own.
  // The setting is in the project menu (#490), kept in the browser.
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-density-compact').click()
  await expect(page.locator('html')).toHaveAttribute('data-density', 'compact')
  await expect(page.getByTestId('track-header').nth(1)).toHaveAttribute('data-height', '28')
  expect(await page.evaluate(() => localStorage.getItem('faulk.density'))).toBe('compact')
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-density-comfortable').click()
  await expect(page.locator('html')).not.toHaveAttribute('data-density', /.*/)
  await expect(page.getByTestId('track-header').nth(1)).toHaveAttribute('data-height', '44')
  // The height is the project's: it stays across a view switch.
  await handle.focus()
  await page.keyboard.press('Shift+ArrowDown')
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('track-header').first().getByTestId('lane-resize')).toHaveAttribute('aria-valuenow', '60')
  expect(errors).toEqual([])
})

test('a row keeps its height with the project: one undoable step, and back after a reload (#490)', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page, 2)
  const first = page.getByTestId('track-header').first()
  const handle = first.getByTestId('lane-resize')
  // A drag is one command, on release: one undo takes the whole drag back.
  const box = (await handle.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 40, { steps: 6 })
  await expect(first).toHaveAttribute('data-height', '84')
  await page.mouse.up()
  await expect(first).toHaveAttribute('data-height', '84')
  await page.keyboard.press('Control+z')
  await expect(first).toHaveAttribute('data-height', '44')
  await page.keyboard.press('Control+Shift+z')
  await expect(first).toHaveAttribute('data-height', '84')
  // Saved with the project (the draft), found again on reopening.
  await expect(page.getByTestId('save-state')).toHaveText(/^Draft saved at/, { timeout: 20_000 })
  await page.reload()
  await expect(page.getByTestId('track-header')).toHaveCount(2)
  await expect(page.getByTestId('track-header').first()).toHaveAttribute('data-height', '84')
  await expect(page.getByTestId('track-header').nth(1)).toHaveAttribute('data-height', '44')
  expect(errors).toEqual([])
})

test('the inspector follows the selection, and the volume is typed on the selected row', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page, 3)
  const inspector = page.getByTestId('track-inspector')
  const rows = page.getByTestId('track-header')
  // One inspector, of the selected track (the last one added), with its volume, pan and devices; no volume or pan in the rows.
  await expect(inspector).toHaveCount(1)
  await page.getByTestId('arr-track').nth(1).click()
  await expect(rows.nth(1)).toHaveAttribute('aria-current', 'true')
  await expect(inspector.getByTestId('inspector-name')).toHaveText('Track 2')
  await expect(inspector.getByTestId('inspector-devices')).toContainText('Drums')
  await expect(inspector.getByRole('slider')).toHaveCount(2)
  await expect(rows.getByRole('slider')).toHaveCount(0)
  await page.getByTestId('arr-track').first().click()
  await expect(inspector.getByTestId('inspector-name')).toHaveText('Synth')
  await expect(inspector.getByTestId('inspector-devices')).toContainText('Oscillator')

  // The volume is shown on hover and on focus of a row, not before.
  const volume = rows.nth(2).getByTestId('lane-volume')
  await page.mouse.move(1000, 10)
  await expect(volume).toBeHidden()
  await rows.nth(2).hover()
  await expect(volume).toHaveText('0.0 dB')
  await page.mouse.move(1000, 10)
  await page.getByTestId('arr-track').nth(2).focus()
  await expect(volume).toBeVisible()

  // An unselected row's digit does nothing; Enter selects it.
  await page.keyboard.press('5')
  await expect(page.getByTestId('value-field')).toHaveCount(0)
  await page.keyboard.press('Enter')
  await expect(rows.nth(2)).toHaveAttribute('aria-current', 'true')
  // Now a digit opens the field with that digit; Enter applies it as one command.
  await page.keyboard.press('-')
  await expect(page.getByTestId('value-input')).toBeFocused()
  await expect(page.getByTestId('value-input')).toHaveValue('-')
  await page.keyboard.type('12')
  mkdirSync(SHOTS, { recursive: true })
  await page.screenshot({ path: `${SHOTS}/lane-volume-typed.png` })
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('value-field')).toHaveCount(0)
  await expect(page.getByTestId('arr-track').nth(2)).toBeFocused()
  await expect(inspector.getByTestId('track-gain-readout')).toHaveText('−12.0')
  await expect(volume).toHaveText('−12.0 dB')
  // Enter opens it on the value; Escape leaves it unchanged; F2 opens it too.
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('value-input')).toHaveValue('−12.0 dB')
  await page.keyboard.press('Escape')
  await page.keyboard.press('F2')
  await page.keyboard.type('-6')
  await page.keyboard.press('Enter')
  await expect(inspector.getByTestId('track-gain-readout')).toHaveText('−6.0')
  // Past the top of the fader: brought back to the bound, and said.
  await page.keyboard.press('9')
  await page.keyboard.type('9')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('value-note')).toBeVisible()
  // The note sits beside the row, on the timeline's side: it covers no other row's name.
  const note = (await page.getByTestId('value-note').boundingBox())!
  const heads = (await page.getByTestId('arr-heads').boundingBox())!
  expect(note.x).toBeGreaterThanOrEqual(heads.x + heads.width)
  await expect(inspector.getByTestId('track-gain-readout')).toHaveText('6.0')
  // A field opened on a row closes, without applying, when the selection moves to another track (an agent's select here, which
  // takes no focus from the field).
  if ((await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) === 'function') {
    await page.keyboard.press('3')
    await expect(page.getByTestId('value-field')).toHaveCount(1)
    const first = await rows.first().getAttribute('data-track')
    await page.evaluate((track) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun({ type: 'view.select_track', track }), first)
    await expect(page.getByTestId('value-field')).toHaveCount(0)
    await page.getByTestId('arr-track').nth(2).click()
    await expect(inspector.getByTestId('track-gain-readout')).toHaveText('6.0')
    await page.getByTestId('arr-track').nth(2).focus()
  }
  // Each entry is one history step.
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(inspector.getByTestId('track-gain-readout')).toHaveText('−6.0')
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(inspector.getByTestId('track-gain-readout')).toHaveText('−12.0')
  await page.screenshot({ path: `${SHOTS}/inspector-and-rows-1440.png` })
  expect(errors).toEqual([])
})

test('the skip link is the first stop and reaches the timeline; Ctrl with plus and minus zooms it', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page, 4)
  // The first stop: Shift+Tab from the top bar's first control lands on it.
  await page.locator('header.top').locator('a, button').first().focus()
  await page.keyboard.press('Shift+Tab')
  const skip = page.getByTestId('skip-link')
  await expect(skip).toBeFocused()
  await expect(skip).toBeInViewport()
  await expect(skip).toHaveText('Skip to the timeline')
  await page.screenshot({ path: `${SHOTS}/skip-link.png` })
  await page.keyboard.press('Enter')
  const grid = page.getByTestId('arr-stage').getByTestId('timegrid')
  await expect(grid).toBeFocused()
  const zoom = async () => Number(await grid.getAttribute('data-px-per-tick'))
  const before = await zoom()
  await page.keyboard.press('Control+Equal')
  await expect.poll(zoom).toBeGreaterThan(before)
  const zoomed = await zoom()
  await page.keyboard.press('Control+Minus')
  await expect.poll(zoom).toBeLessThan(zoomed)
  // The page itself was not zoomed.
  expect(await page.evaluate(() => Math.round(window.devicePixelRatio * 100))).toBe(100)
  // Bare plus and minus still zoom the focused grid.
  const now = await zoom()
  await page.keyboard.press('Equal')
  await expect.poll(zoom).toBeGreaterThan(now)

  // In another view it reaches the main view.
  await page.getByTestId('nav-mixer').click()
  await page.locator('header.top').locator('a, button').first().focus()
  await page.keyboard.press('Shift+Tab')
  await expect(skip).toBeFocused()
  await expect(skip).toHaveText('Skip to the main view')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('view')).toBeFocused()
  expect(errors).toEqual([])
})

test('a row is dragged by its colour bar to reorder the tracks', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page, 3)
  const order = () => page.getByTestId('track-header').evaluateAll((rows) => rows.map((r) => r.getAttribute('data-track')))
  const [one, two, three] = await order()
  await page.getByTestId('track-grip').nth(2).dragTo(page.getByTestId('track-header').first())
  await expect.poll(order).toEqual([three, one, two])
  expect(errors).toEqual([])
})
