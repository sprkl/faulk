import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #473: named section markers on the ruler, jumped to with Alt+Page Up/Down, added with Alt+K; the whole song in
// one zoom (Alt+Z); the arrangement opens on 16 to 32 bars.

test.describe.configure({ timeout: 120_000 })
test.use({ storageState: { cookies: [], origins: [] } })

const BAR = 3840
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
type View = { project: { tracks: Record<string, unknown>; markers?: Record<string, { tick: number; name: string }> } }
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())
const markerNames = async (page: Page) =>
  Object.values((await view(page)).project.markers ?? {})
    .sort((a, b) => a.tick - b.tick)
    .map((m) => `${m.name}@${m.tick / BAR + 1}`)

async function start(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
}

const grid = (page: Page) => page.getByTestId('arr-stage').getByTestId('timegrid')
/** How many bars the window shows. */
const barsShown = async (page: Page) => {
  const ppt = Number(await grid(page).getAttribute('data-px-per-tick'))
  const width = (await page.getByTestId('arr-stage').boundingBox())!.width
  return width / (ppt * BAR)
}

test('section markers: added, named, jumped to by keys, renamed and deleted, all undoable', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the command hook exists in the dev server only')
  await expect.poll(async () => Number(await grid(page).getAttribute('data-draw-count'))).toBeGreaterThan(0)

  // The arrangement opens on 16 to 32 bars (an empty song: 16).
  expect(Math.round(await barsShown(page))).toBe(16)

  // An agent places two markers; the button adds a third at the playhead's bar (bar 1).
  await run(page, { type: 'arrangement.marker.add', tick: BAR * 8, name: 'Verse' })
  await run(page, { type: 'arrangement.marker.add', tick: BAR * 16, name: 'Chorus' })
  await page.getByTestId('arr-add-marker').click()
  await expect.poll(() => markerNames(page)).toEqual(['Section 1@1', 'Verse@9', 'Chorus@17'])
  await expect(page.getByTestId('arr-marker').filter({ hasText: 'Verse' })).toHaveAccessibleName('Marker Verse, bar 9')

  // Alt+Page Down twice: the playhead jumps to the verse, then the chorus; Alt+Page Up comes back.
  const position = page.getByTestId('transport-position')
  await page.keyboard.press('Alt+PageDown')
  await expect(page.getByTestId('arr-said')).toHaveText(/^Marker Verse, bar 9/)
  await page.keyboard.press('Alt+PageDown')
  await expect(page.getByTestId('arr-said')).toHaveText(/^Marker Chorus, bar 17/)
  await page.keyboard.press('Alt+PageUp')
  await expect(page.getByTestId('arr-said')).toHaveText(/^Marker Verse, bar 9/)
  await expect(position).toContainText('9.1')

  // Alt+K adds one at the playhead's bar.
  await page.keyboard.press('Alt+K')
  await expect.poll(() => markerNames(page)).toEqual(['Section 1@1', 'Verse@9', 'Section 2@9', 'Chorus@17'])

  // Double-click renames in place; Delete removes; undo brings it back. The whole song first: every marker in sight.
  await page.getByTestId('arr-fit-song').click()
  await page.getByTestId('arr-marker').filter({ hasText: 'Section 1' }).dblclick()
  await page.getByTestId('arr-marker-field').fill('Intro')
  await page.keyboard.press('Enter')
  await expect.poll(() => markerNames(page)).toEqual(['Intro@1', 'Verse@9', 'Section 2@9', 'Chorus@17'])
  await page.getByTestId('arr-marker').filter({ hasText: 'Section 2' }).focus()
  await page.keyboard.press('Delete')
  await expect.poll(() => markerNames(page)).toEqual(['Intro@1', 'Verse@9', 'Chorus@17'])
  await page.keyboard.press('Control+z')
  await expect.poll(() => markerNames(page)).toEqual(['Intro@1', 'Verse@9', 'Section 2@9', 'Chorus@17'])

  // The history names them.
  await page.getByTestId('panel-history').click()
  await expect(page.getByTestId('history-entry').filter({ hasText: 'Rename marker Section 1 to Intro' })).toHaveCount(1)
  expect(errors).toEqual([])
})

test('the right click on a marker: go to, rename, move by the grid step, delete (#530)', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the command hook exists in the dev server only')
  await expect.poll(async () => Number(await grid(page).getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await run(page, { type: 'arrangement.marker.add', tick: BAR * 4, name: 'Verse' })
  const marker = page.getByTestId('arr-marker').filter({ hasText: 'Verse' })
  const menu = page.getByTestId('context-menu')

  // Go to: the playhead reaches the marker's bar.
  await marker.click({ button: 'right' })
  await expect(menu).toHaveAttribute('aria-label', 'Marker Verse')
  await expect(page.getByTestId('ctx-marker-jump')).toBeFocused()
  await page.getByTestId('ctx-marker-jump').click()
  await expect(page.getByTestId('transport-position')).toContainText('5.1')

  // Later, then earlier: one grid step each, one command each.
  await marker.click({ button: 'right' })
  await page.getByTestId('ctx-marker-later').click()
  await expect.poll(async () => (await markerNames(page))[0]).not.toBe('Verse@5')
  await marker.click({ button: 'right' })
  await page.getByTestId('ctx-marker-earlier').click()
  await expect.poll(() => markerNames(page)).toEqual(['Verse@5'])

  // Rename opens the field in place.
  await marker.click({ button: 'right' })
  await page.getByTestId('ctx-marker-rename').click()
  await page.getByTestId('arr-marker-field').fill('Drop')
  await page.keyboard.press('Enter')
  await expect.poll(() => markerNames(page)).toEqual(['Drop@5'])

  // Delete, then undo brings it back.
  await page.getByTestId('arr-marker').filter({ hasText: 'Drop' }).click({ button: 'right' })
  await page.getByTestId('ctx-marker-delete').click()
  await expect.poll(() => markerNames(page)).toEqual([])
  await page.keyboard.press('Control+z')
  await expect.poll(() => markerNames(page)).toEqual(['Drop@5'])
  expect(errors).toEqual([])
})

test('the whole song in one zoom, and a long song opens on 32 bars', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the command hook exists in the dev server only')
  const track = Object.keys((await view(page)).project.tracks)[0]!
  await run(page, { type: 'clip.create', track, at: { at: 'arrangement', start: BAR * 59 } })
  await expect.poll(async () => Number(await grid(page).getAttribute('data-draw-count'))).toBeGreaterThan(0)

  // Alt+Z: 60 bars and one more across the window.
  await page.keyboard.press('Alt+Z')
  await expect.poll(async () => Math.round(await barsShown(page))).toBe(61)
  // Zoom in, then the button brings the whole song back.
  await page.getByTestId('arr-zoom-in').click()
  await page.getByTestId('arr-fit-song').click()
  await expect.poll(async () => Math.round(await barsShown(page))).toBe(61)
  expect(errors).toEqual([])
})

test('Ctrl+G puts the playhead on a bar or a bar.beat from any view; Alt+arrows move a focused marker one grid step (#554)', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  await expect.poll(async () => Number(await grid(page).getAttribute('data-draw-count'))).toBeGreaterThan(0)
  const position = page.getByTestId('transport-position')
  const dialog = page.getByTestId('goto-dialog')

  // A bar and a beat; the field is focused, Enter goes, the dialog closes.
  await page.keyboard.press('Control+g')
  await expect(dialog).toBeVisible()
  await expect(page.getByTestId('goto-field')).toBeFocused()
  await page.keyboard.type('5.3')
  await page.keyboard.press('Enter')
  await expect(dialog).toBeHidden()
  await expect(position).toContainText('5.3')

  // A text that means nothing is said and keeps the dialog open; Escape closes it and the playhead stays.
  await page.keyboard.press('Control+g')
  await page.keyboard.type('5.9')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('goto-error')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(dialog).toBeHidden()
  await expect(position).toContainText('5.3')

  // In another view too (the mixer), a bar alone goes to its first beat.
  await page.keyboard.press('Alt+3')
  await page.keyboard.press('Control+g')
  await page.keyboard.type('2')
  await page.keyboard.press('Enter')
  await expect(position).toContainText('2.1')

  // The playhead placed by Ctrl+G is where the keyboard acts: Alt+K adds a marker at that bar, not at the old stop position.
  await page.keyboard.press('Alt+1')
  await page.keyboard.press('Control+g')
  await page.keyboard.type('7.2')
  await page.keyboard.press('Enter')
  await expect(position).toContainText('7.2')
  await page.keyboard.press('Alt+K')
  await expect(page.getByTestId('arr-marker')).toHaveCount(1)
  await expect(page.getByTestId('arr-marker')).toHaveAccessibleName(/bar 7$/)

  // A focused marker moves by the snap step, right and back; its name is in the live region.
  test.skip((await hooks(page)) !== 'function', 'the command hook exists in the dev server only')
  const tickOf = async () => Object.values((await view(page)).project.markers ?? {}).map((m) => m.tick)
  const [before] = await tickOf()
  await page.getByTestId('arr-marker').focus()
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(async () => (await tickOf())[0]).toBeGreaterThan(before!)
  await expect(page.getByTestId('arr-said')).toHaveText(/Marker Section 1 moved to bar/)
  await page.keyboard.press('Alt+ArrowLeft')
  await expect.poll(async () => (await tickOf())[0]).toBe(before)
  await expect(page.getByTestId('arr-marker')).toBeFocused()
  expect(errors).toEqual([])
})
