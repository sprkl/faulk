import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #915: "Lengthen to..." (Ctrl+Shift+L) fixes the end of a clip at the end of a bar in one action: one `arrangement.resize`, one
// history entry, one undo. The content repeats; a clip in the way refuses it, said and named, and nothing changes. Real worklet,
// engine and project core, read back from the project; the console stays clean.

test.use({ storageState: { cookies: [], origins: [] } })

const RULER = 28
const LANE = 44
const BAR = 3840
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
const done = (page: Page) => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[] } }).__faulkHistory().undo.length)
type Block = { id: string; row: number; start: number; length: number; clip: string }
const blocks = async (page: Page): Promise<Block[]> => JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')

async function start(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await page.getByTestId('add-track').click()
  await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Drums', exact: true }).click()
  await expect(page.getByTestId('track-header')).toHaveCount(2)
  await expect.poll(async () => Number(await page.getByTestId('arr-stage').getByTestId('timegrid').getAttribute('data-px-per-tick'))).toBeGreaterThan(0)
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
}

/** A clip of one bar by a double click on a track's row, at a bar. */
async function clipAt(page: Page, row: number, barIndex: number) {
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const before = (await blocks(page)).length
  await page.mouse.dblclick(box.x + (barIndex * BAR + BAR / 2) * ppt, box.y + RULER + row * LANE + LANE / 2)
  await expect.poll(async () => (await blocks(page)).length).toBe(before + 1)
}

async function focusClip(page: Page, row: number, barIndex: number) {
  const target = (await blocks(page)).find((b) => b.row === row && b.start === barIndex * BAR)!
  await page.locator(`[data-testid=arr-item][data-block="${target.id}"]`).focus()
}

test('Ctrl+Shift+L lengthens a one-bar clip to the end of bar 25 in one action, and one undo gives it back', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkHistory?: unknown }).__faulkHistory)) !== 'function', 'the dev server only')
  const said = page.getByTestId('arr-said')

  await clipAt(page, 0, 0)
  // A clip with a note repeats its content when its placement is longer (an empty clip would grow with it, #898).
  const clip = (await blocks(page))[0]!.clip
  await run(page, { type: 'clip.notes.add', clip, notes: [{ start: 0, pitch: 60, length: BAR / 4, velocity: 100, cents: 0, chance: 100, repeat: 1 }] })
  await focusClip(page, 0, 0)
  const entries = await done(page)

  await page.keyboard.press('Control+Shift+l')
  const dialog = page.getByTestId('lengthen-dialog')
  await expect(dialog).toBeVisible()
  await expect(dialog).toHaveAccessibleName('Lengthen to')
  await expect(page.getByTestId('lengthen-field')).toBeFocused()
  // A bar the clip already covers is refused in place.
  await page.getByTestId('lengthen-field').fill('1')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('lengthen-error')).toBeVisible()
  await page.getByTestId('lengthen-field').fill('25')
  await page.keyboard.press('Enter')

  await expect(dialog).toBeHidden()
  await expect.poll(async () => (await blocks(page)).map((b) => b.start + b.length)[0]!).toBe(25 * BAR)
  await expect(said).toHaveText('Lengthened to the end of bar 25')
  expect(await done(page)).toBe(entries + 1)
  expect((await blocks(page)).length).toBe(1)
  // The focus is back on the clip.
  await expect(page.locator('[data-testid=arr-item]').first()).toBeFocused()

  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start + b.length)[0]!).toBe(BAR)
  expect(await done(page)).toBe(entries)
  expect(errors).toEqual([])
})

test('Lengthen to is in the More menu, with its shortcut', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await clipAt(page, 0, 0)
  await focusClip(page, 0, 0)
  await page.getByTestId('arr-more').click()
  const entry = page.getByTestId('ctx-more-lengthen')
  await expect(entry).toBeVisible()
  await expect(entry).toHaveText(/Lengthen to…/)
  await expect(entry.locator('kbd')).toHaveText(/L$/)
  await page.keyboard.press('Escape')
  expect(errors).toEqual([])
})

test('a clip in the way refuses the lengthening, names it, and changes nothing', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkHistory?: unknown }).__faulkHistory)) !== 'function', 'the dev server only')
  const said = page.getByTestId('arr-said')
  await clipAt(page, 1, 0)
  await clipAt(page, 1, 3)
  const before = JSON.stringify(await blocks(page))
  const entries = await done(page)
  await focusClip(page, 1, 0)
  await page.keyboard.press('Control+Shift+l')
  await page.getByTestId('lengthen-field').fill('6')
  await page.keyboard.press('Enter')
  await expect(said).toHaveText(/^Not lengthened: .*, bars 4 to 4, is in the way/)
  expect(JSON.stringify(await blocks(page))).toBe(before)
  expect(await done(page)).toBe(entries)

  // Up to the bar before the other clip is free ground.
  await page.keyboard.press('Control+Shift+l')
  await page.getByTestId('lengthen-field').fill('3')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await blocks(page)).filter((b) => b.start === 0).map((b) => b.start + b.length)[0]!).toBe(3 * BAR)
  expect(await done(page)).toBe(entries + 1)
  expect(errors).toEqual([])
})

test('an empty clip grows with its placement (said), and a clip that ends inside a bar can be given the end of that bar', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkHistory?: unknown }).__faulkHistory)) !== 'function', 'the dev server only')
  const said = page.getByTestId('arr-said')
  await clipAt(page, 0, 0)
  await focusClip(page, 0, 0)
  await page.keyboard.press('Control+Shift+l')
  await page.getByTestId('lengthen-field').fill('3')
  await page.keyboard.press('Enter')
  await expect(said).toHaveText('Lengthened to the end of bar 3, clip included')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start + b.length)[0]!).toBe(3 * BAR)

  // Ends in the middle of bar 2 after a shortening: the field starts at bar 2, the end of the bar holding the clip's end.
  const id = (await blocks(page))[0]!.id
  await run(page, { type: 'arrangement.resize', placement: id, edge: 'end', to: BAR + BAR / 2 })
  await expect.poll(async () => (await blocks(page)).map((b) => b.start + b.length)[0]!).toBe(BAR + BAR / 2)
  await focusClip(page, 0, 0)
  await page.keyboard.press('Control+Shift+l')
  await expect(page.getByTestId('lengthen-field')).toHaveValue('2')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start + b.length)[0]!).toBe(2 * BAR)
  expect(errors).toEqual([])
})
