import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #575: the arrangement toolbar keeps the actions of every step of making a track in view and puts the others in a "More"
// menu (the shared context menu, with the shortcuts written). The bar stays on one line, in English and in French, from 1280 px.
// The menu is used from the keyboard alone.

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const arr = (page: Page) => page.getByTestId('arrangement')
const toolbar = (page: Page) => page.getByRole('toolbar', { name: /Edit the selection|Modifier la sélection/ })

/** A page with one clip in the arrangement, all clips selected (the bar then shows its longest texts). */
async function open(page: Page, width: number) {
  await page.setViewportSize({ width, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await engineUp(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toHaveAttribute('data-block-count', '1')
  const cell = page.locator('[data-testid=arr-item]').first()
  await cell.focus()
  await page.keyboard.press('Control+a')
  await expect(arr(page)).toHaveAttribute('data-selected-count', '1')
}

/** Where the toolbar's controls sit: one row when they share their middle line, and none leaves the bar. */
async function rows(page: Page) {
  return toolbar(page).evaluate((bar) => {
    const box = bar.getBoundingClientRect()
    const middles = [...bar.children].map((el) => {
      const r = el.getBoundingClientRect()
      return { middle: Math.round(r.top + r.height / 2), right: r.right, left: r.left, name: el.getAttribute('data-testid') ?? el.className }
    })
    return {
      height: box.height,
      lines: new Set(middles.map((m) => m.middle)).size,
      overflow: bar.scrollWidth - bar.clientWidth,
      outside: middles.filter((m) => m.right > box.right + 0.5 || m.left < box.left - 0.5).map((m) => m.name),
    }
  })
}

for (const [locale, width] of [
  ['en-US', 1280],
  ['en-US', 1920],
  ['fr-FR', 1280],
  ['fr-FR', 1920],
] as const) {
  test(`the arrangement toolbar is one line at ${width} px in ${locale} (#575)${locale === 'fr-FR' && width === 1280 ? ' @full' : ''}`, async ({ browser }, info) => {
    const context = await browser.newContext({ locale, baseURL: info.project.use.baseURL })
    const page = await context.newPage()
    const seen = watch(page)
    await open(page, width)

    const bar = await rows(page)
    expect(bar.lines, `one line (${JSON.stringify(bar)})`).toBe(1)
    expect(bar.overflow).toBe(0)
    expect(bar.outside).toEqual([])
    // The same height as with nothing selected: the lanes below do not move when a selection comes and goes.
    const before = (await arr(page).getByTestId('arr-stage').boundingBox())!.y
    await page.keyboard.press('Escape')
    expect((await arr(page).getByTestId('arr-stage').boundingBox())!.y).toBe(before)
    // The main actions are in the bar; the secondary ones are not.
    for (const id of ['arr-snap', 'arr-split', 'arr-duplicate', 'arr-delete', 'arr-zoom-out', 'arr-zoom-in', 'arr-fit-song', 'arr-follow', 'arr-more']) {
      await expect(page.getByTestId(id)).toBeVisible()
    }
    for (const id of ['arr-duplicate-linked', 'arr-insert-bars', 'arr-delete-bars', 'arr-add-marker', 'arr-curve-linear', 'arr-curve-step', 'arr-bars']) {
      await expect(page.getByTestId(id)).toHaveCount(0)
    }

    expect(seen.errors).toEqual([])
    await context.close()
  })
}

for (const [locale, repeat] of [
  ['en-US', 'Repeat until…'],
  ['fr-FR', 'Répéter jusqu’à…'],
] as const) {
  test(`the More menu is used from the keyboard, with its shortcuts written (${locale}) (#575)${locale === 'fr-FR' ? ' @full' : ''}`, async ({ browser }, info) => {
    const context = await browser.newContext({ locale, baseURL: info.project.use.baseURL })
    const page = await context.newPage()
    const seen = watch(page)
    await open(page, 1280)
    const more = page.getByTestId('arr-more')
    const menu = page.getByTestId('context-menu')

    // Enter on the button opens the menu under it, the first entry takes the focus; the shortcuts are written.
    await more.focus()
    await expect(more).toHaveAttribute('aria-haspopup', 'menu')
    await expect(more).toHaveAttribute('aria-expanded', 'false')
    await page.keyboard.press('Enter')
    await expect(menu).toBeVisible()
    await expect(more).toHaveAttribute('aria-expanded', 'true')
    await expect(page.getByTestId('ctx-more-duplicate-linked')).toBeFocused()
    await expect(page.getByTestId('ctx-more-duplicate-linked').locator('kbd')).toHaveText(/Shift\+D|Maj\+D|⇧D/)
    await expect(page.getByTestId('ctx-more-repeat')).toHaveText(new RegExp(repeat))
    await expect(page.getByTestId('ctx-more-repeat').locator('kbd')).toHaveText(/R$/)
    await expect(page.getByTestId('ctx-more-marker').locator('kbd')).toHaveText(/K$/)
    // With no time or point selected, those entries are off and the arrows skip them.
    await expect(page.getByTestId('ctx-more-delete-bars')).toBeDisabled()
    await expect(page.getByTestId('ctx-more-curve-step')).toBeDisabled()

    // Escape closes it and the focus is back on the button.
    await page.keyboard.press('Escape')
    await expect(menu).toHaveCount(0)
    await expect(more).toBeFocused()
    await expect(more).toHaveAttribute('aria-expanded', 'false')

    // The arrows reach "Repeat until…" and Enter opens its field; Escape closes the field, the focus is back on the button.
    await page.keyboard.press('Space')
    await page.keyboard.press('ArrowDown')
    await expect(page.getByTestId('ctx-more-repeat')).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('repeat-dialog')).toBeVisible()
    await page.getByTestId('repeat-field').fill('5')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('repeat-dialog')).toBeHidden()
    await expect.poll(async () => Number(await arr(page).getAttribute('data-block-count'))).toBeGreaterThan(1)

    // End reaches the last enabled entry (the marker: the curves and the bar deletion are off); Enter adds a marker at the playhead.
    await more.focus()
    await page.keyboard.press('Enter')
    await page.keyboard.press('End')
    await expect(page.getByTestId('ctx-more-marker')).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(menu).toHaveCount(0)
    await expect(page.getByTestId('arr-marker')).toHaveCount(1)

    // Insert bars: the entry opens its field; a wrong count is refused in place and a good one moves the clips.
    await more.focus()
    await page.keyboard.press('Enter')
    // (the arrows skip the entries that are off: after the repeat, several clips may be selected)
    for (let i = 0; i < 8 && !(await page.getByTestId('ctx-more-insert-bars').evaluate((el) => el === document.activeElement)); i++) {
      await page.keyboard.press('ArrowDown')
    }
    await expect(page.getByTestId('ctx-more-insert-bars')).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('insert-dialog')).toBeVisible()
    await page.getByTestId('insert-field').fill('0')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('insert-error')).toBeVisible()
    await page.getByTestId('insert-field').fill('2')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('insert-dialog')).toBeHidden()
    const starts = (JSON.parse((await arr(page).getAttribute('data-blocks')) ?? '[]') as { start: number }[]).map((b) => b.start)
    expect(Math.min(...starts)).toBe(2 * 3840)

    expect(seen.errors).toEqual([])
    await context.close()
  })
}
