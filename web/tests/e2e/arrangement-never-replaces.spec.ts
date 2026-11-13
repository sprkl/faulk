import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #805: a move, a copy or a placement from the keyboard or the buttons never replaces a clip without a word. Alt+Up and Alt+Down skip the tracks whose place
// is taken and land on the next free one (or refuse, saying so); Ctrl+D refuses a copy that would land on the clip after it. Real
// worklet, engine and project core: every edit is a command, read back from the project, and the console stays clean.

test.use({ storageState: { cookies: [], origins: [] } })

const RULER = 28
const LANE = 44
const done = (page: Page) => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[] } }).__faulkHistory().undo.length)
const blocks = async (page: Page): Promise<{ id: string; row: number; start: number }[]> =>
  JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')
const layout = async (page: Page) =>
  (await blocks(page))
    .map((b) => `${b.row}@${b.start}`)
    .sort()
    .join(' ')

async function start(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  for (let i = 0; i < 2; i++) {
    await page.getByTestId('add-track').click()
    await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Drums', exact: true }).click()
  }
  await expect(page.getByTestId('track-header')).toHaveCount(3)
  await expect.poll(async () => Number(await page.getByTestId('arr-stage').getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
}

/** A clip of one bar by a double click on a track's row, at a bar. */
async function clipAt(page: Page, row: number, barIndex: number) {
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const bar = 3840
  const before = (await blocks(page)).length
  await page.mouse.dblclick(box.x + (barIndex * bar + bar / 2) * ppt, box.y + RULER + row * LANE + LANE / 2)
  await expect.poll(async () => (await blocks(page)).length).toBe(before + 1)
}

/** Puts the keyboard focus on the clip of a track at a bar. */
async function focusClip(page: Page, row: number, barIndex: number) {
  const target = (await blocks(page)).find((b) => b.row === row && b.start === barIndex * 3840)!
  await page.locator(`[data-testid=arr-item][data-block="${target.id}"]`).focus()
}

test('Alt+Down skips the track whose place is taken, refuses when none is free, and Ctrl+D refuses to copy over the next clip', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkHistory?: unknown }).__faulkHistory)) !== 'function', 'the dev server only')
  const said = page.getByTestId('arr-said')

  // Track 1 holds a clip at bar 1 and track 2 too; track 3 is free there.
  await clipAt(page, 0, 0)
  await clipAt(page, 1, 0)
  await expect.poll(() => layout(page)).toBe('0@0 1@0')
  await focusClip(page, 0, 0)
  const entries = await done(page)
  await page.keyboard.press('Alt+ArrowDown')
  // It goes over track 2, which is taken, onto track 3, and the clip of track 2 is still there.
  await expect.poll(() => layout(page)).toBe('1@0 2@0')
  await expect(said).toHaveText(/ is taken, moved to /)
  expect(await done(page)).toBe(entries + 1)
  await page.keyboard.press('Control+z')
  await expect.poll(() => layout(page)).toBe('0@0 1@0')

  // Every track below is taken now: the move is refused, says so, and changes nothing.
  await clipAt(page, 2, 0)
  await expect.poll(() => layout(page)).toBe('0@0 1@0 2@0')
  await focusClip(page, 0, 0)
  const full = await done(page)
  await page.keyboard.press('Alt+ArrowDown')
  await expect(said).toHaveText(/No free track that way/)
  expect(await layout(page)).toBe('0@0 1@0 2@0')
  expect(await done(page)).toBe(full)
  // The same going up from the last track.
  await focusClip(page, 2, 0)
  await page.keyboard.press('Alt+ArrowUp')
  await expect(said).toHaveText(/No free track that way/)
  expect(await layout(page)).toBe('0@0 1@0 2@0')

  // Ctrl+D copies a clip to the bar after it when that place is free, and refuses when a clip sits there.
  await clipAt(page, 0, 1)
  await expect.poll(() => layout(page)).toBe('0@0 0@3840 1@0 2@0')
  await focusClip(page, 0, 0)
  const before = await done(page)
  await page.keyboard.press('Control+d')
  await expect(said).toHaveText(/Not copied/)
  expect(await layout(page)).toBe('0@0 0@3840 1@0 2@0')
  expect(await done(page)).toBe(before)
  // A click on an empty lane drops the selection the refused copy left, so that Ctrl+D copies the focused clip.
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.click(box.x + 5 * 3840 * ppt, box.y + RULER + 2 * LANE + LANE / 2)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '0')
  await focusClip(page, 0, 1)
  await page.keyboard.press('Control+d')
  await expect.poll(() => layout(page)).toBe('0@0 0@3840 0@7680 1@0 2@0')
  expect(await done(page)).toBe(before + 1)

  expect(errors).toEqual([])
})

const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)

test('Alt+Left, Alt+Right and Repeat until refuse to cut a neighbour and name it, and a clip off the bar lines says its beats', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the dev server only')
  const said = page.getByTestId('arr-said')

  // Two clips of a bar side by side on the first track.
  await clipAt(page, 0, 0)
  await clipAt(page, 0, 1)
  await expect.poll(() => layout(page)).toBe('0@0 0@3840')
  const entries = await done(page)

  // Alt+Right on the first would land on the second, Alt+Left on the second on the first: nothing moves, the message names the other.
  await focusClip(page, 0, 0)
  await page.keyboard.press('Alt+ArrowRight')
  await expect(said).toHaveText(/^Not moved: .*, bars 2 to 2, is in the way/)
  expect(await layout(page)).toBe('0@0 0@3840')
  await focusClip(page, 0, 1)
  await page.keyboard.press('Alt+ArrowLeft')
  await expect(said).toHaveText(/^Not moved: .*, bars 1 to 1, is in the way/)
  expect(await layout(page)).toBe('0@0 0@3840')
  expect(await done(page)).toBe(entries)

  // Repeat until bar 4 from the first clip would cover the second: refused, said, nothing changed.
  await focusClip(page, 0, 0)
  await page.keyboard.press('Control+Shift+r')
  await expect(page.getByTestId('repeat-dialog')).toBeVisible()
  await page.getByTestId('repeat-field').fill('4')
  await page.keyboard.press('Enter')
  await expect(said).toHaveText(/^Not repeated: .*, bars 2 to 2, is in the way/)
  expect(await layout(page)).toBe('0@0 0@3840')
  expect(await done(page)).toBe(entries)

  // Free ground still works: the second clip moves one step to the right.
  await focusClip(page, 0, 1)
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(async () => (await layout(page)) !== '0@0 0@3840').toBe(true)
  expect(await done(page)).toBe(entries + 1)

  // A clip that starts and ends off the bar lines says its beats: bar 5 beat 2 to bar 5 beat 3, and one that ends on a bar line while it starts off it.
  await clipAt(page, 2, 4)
  const lone = (await blocks(page)).find((b) => b.row === 2 && b.start === 4 * 3840)!
  await run(page, { type: 'arrangement.move', placements: [lone.id], by: 960 })
  await run(page, { type: 'arrangement.resize', placement: lone.id, edge: 'end', to: 4 * 3840 + 1920 })
  const cell = page.locator(`[data-testid=arr-item][data-block="${lone.id}"]`)
  await expect(cell).toHaveAttribute('aria-label', /bar 5 beat 2 to 5 beat 3/)
  await run(page, { type: 'arrangement.resize', placement: lone.id, edge: 'end', to: 5 * 3840 })
  await expect(cell).toHaveAttribute('aria-label', /bar 5 beat 2 to 6 beat 1/)

  expect(errors).toEqual([])
})

test('Ctrl+D copies the focused clip on its own track even when a clip of another track is selected', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the dev server only')

  await clipAt(page, 0, 0)
  await clipAt(page, 1, 0)
  await expect.poll(() => layout(page)).toBe('0@0 1@0')
  // Select the clip of the second track, then put the keyboard on the first track's clip.
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.click(box.x + (3840 / 2) * ppt, box.y + RULER + 1 * LANE + LANE / 2)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '1')
  await focusClip(page, 0, 0)
  const before = await done(page)
  await page.keyboard.press('Control+d')
  await expect.poll(() => layout(page)).toBe('0@0 0@3840 1@0')
  expect(await done(page)).toBe(before + 1)

  expect(errors).toEqual([])
})
