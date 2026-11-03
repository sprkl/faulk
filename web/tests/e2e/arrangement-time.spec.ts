import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #396: insert and delete bars across all tracks, in a real browser, in English and in French. "Insert bars"
// opens empty bars at the play position's bar (or at a time selection), a Shift+drag on the ruler selects time,
// "Delete selected bars" closes it up, and each action is one undo.

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

/** The toolbar's "More" menu (#575): opens it and returns the entry of that id. */
async function more(page: Page, id: string) {
  await page.getByTestId('arr-more').click()
  return page.getByTestId(`ctx-more-${id}`)
}
/** Inserts `count` bars through the menu and the field it opens. */
async function insertBars(page: Page, count: number) {
  await (await more(page, 'insert-bars')).click()
  await page.getByTestId('insert-field').fill(String(count))
  await page.getByTestId('insert-submit').click()
}
/** Whether "Delete selected bars" can be used now: the menu is opened to look, then closed. */
async function deleteBarsEnabled(page: Page): Promise<boolean> {
  const entry = await more(page, 'delete-bars')
  const enabled = await entry.isEnabled()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('context-menu')).toHaveCount(0)
  return enabled
}

/** Shift+drag on the ruler from one bar boundary to another: a time selection. */
async function selectTime(page: Page, fromBar: number, toBar: number) {
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const y = box.y + RULER / 2
  await page.keyboard.down('Shift')
  await page.mouse.move(box.x + fromBar * BAR * ppt + 1, y)
  await page.mouse.down()
  await page.mouse.move(box.x + ((fromBar + toBar) / 2) * BAR * ppt, y, { steps: 4 })
  await page.mouse.move(box.x + toBar * BAR * ppt, y, { steps: 4 })
  await page.mouse.up()
  await page.keyboard.up('Shift')
}

for (const [locale, insertLabel, deleteLabel] of [
  ['en-US', 'Insert bars…', 'Delete selected bars'],
  ['fr-FR', 'Insérer des mesures…', 'Supprimer les mesures sélectionnées'],
] as const) {
  test(`arrangement view (${locale}): insert bars, select time with Shift+drag, delete the selected bars, each one undo${locale === 'fr-FR' ? ' @full' : ''}`, async ({
    browser,
  }, info) => {
    test.setTimeout(120_000)
    const context = await browser.newContext({ locale, baseURL: info.project.use.baseURL })
    const page = await context.newPage()
    const seen = watch(page)
    await page.setViewportSize({ width: 1800, height: 900 })
    await page.goto('/app')
    await startAudio(page)
    await engineUp(page)

    await page.getByTestId('panel-piano-roll').click()
    await page.getByTestId('roll-create-clip').click()
    await page.getByTestId('nav-arrangement').click()
    await expect(arr(page)).toHaveAttribute('data-block-count', '1')
    expect(await starts(page)).toEqual([0])
    await page.getByTestId('arr-snap').selectOption('bar')

    // The actions speak the page language.
    await expect(await more(page, 'insert-bars')).toHaveText(insertLabel)
    await expect(page.getByTestId('ctx-more-delete-bars')).toHaveText(deleteLabel)
    await page.keyboard.press('Escape')

    // Two bars inserted at the play position (bar 1): the clip moves two bars later.
    await insertBars(page, 2)
    await expect.poll(() => starts(page)).toEqual([2 * BAR])
    await page.keyboard.press('Control+z')
    await expect.poll(() => starts(page)).toEqual([0])
    await page.keyboard.press('Control+Shift+z')
    await expect.poll(() => starts(page)).toEqual([2 * BAR])

    // Select the two empty bars with Shift+drag on the ruler, delete them: the clip is back at the start.
    expect(await deleteBarsEnabled(page)).toBe(false)
    await selectTime(page, 0, 2)
    expect(await deleteBarsEnabled(page)).toBe(true)
    await (await more(page, 'delete-bars')).click()
    await expect.poll(() => starts(page)).toEqual([0])
    expect(await deleteBarsEnabled(page)).toBe(false)
    await page.keyboard.press('Control+z')
    await expect.poll(() => starts(page)).toEqual([2 * BAR])

    // One bar inserted at the start of a time selection (bar 2), not at the play position: the clip, which
    // starts after that point, moves one bar later; the selection is spent.
    await selectTime(page, 1, 2)
    await insertBars(page, 1)
    await expect.poll(() => starts(page)).toEqual([3 * BAR])
    expect(await deleteBarsEnabled(page)).toBe(false)

    // The selection stays after an undo and a redo (#435) ...
    await selectTime(page, 0, 1)
    expect(await deleteBarsEnabled(page)).toBe(true)
    await page.keyboard.press('Control+z')
    await page.keyboard.press('Control+Shift+z')
    expect(await deleteBarsEnabled(page)).toBe(true)
    // ... but a project started on top of it drops it, so that Delete never targets a range chosen on another project (#445).
    // The command hook exists on the dev server only (#466).
    if (info.project.name === 'dev') {
      await page.evaluate(() => (window as unknown as { __faulkRun(c: object): void }).__faulkRun({ type: 'project.new' }))
      await expect(arr(page)).toHaveAttribute('data-block-count', '0')
      expect(await deleteBarsEnabled(page)).toBe(false)
    }

    expect(seen.errors).toEqual([])
    await context.close()
  })
}
