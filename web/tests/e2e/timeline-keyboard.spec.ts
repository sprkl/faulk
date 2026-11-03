import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #471: the timeline and the piano roll from the keyboard and for a screen reader. The canvas has named cells over it
// (a grid, one cell in the tab order); the arrows move between clips or notes, Shift+arrows extend the selection,
// Alt+arrows move by the snap, Enter opens (a clip) or sounds (a note), Delete removes; a polite live region says what
// an edit did. Real worklet, engine and project core: every edit is a command, read back from the project.

test.use({ storageState: { cookies: [], origins: [] } })

const SHOTS = '../.qa/design-review-2026-11-01/after-471'
const RULER = 28
const LANE = 44
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
/** How many entries the history holds (done, not undone): one key, one entry. */
const done = (page: Page) => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[] } }).__faulkHistory().undo.length)
const blocks = async (page: Page): Promise<{ id: string; row: number; start: number }[]> =>
  JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')

async function start(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  for (let i = 0; i < 2; i++) {
    await page.getByTestId('add-track').click()
    await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Drums', exact: true }).click()
  }
  await expect(page.getByTestId('track-header')).toHaveCount(3)
  await expect.poll(async () => Number(await page.getByTestId('arr-stage').getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
}

/** A clip of one bar by a double click on a track's row, at a bar (the grid is at its default zoom). */
async function clipAt(page: Page, row: number, barIndex: number) {
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const bar = 3840
  await page.mouse.dblclick(box.x + (barIndex * bar + bar / 2) * ppt, box.y + RULER + row * LANE + LANE / 2)
}

test('the clips are named cells, reached by the arrows, moved by Alt+arrows, opened by Enter, removed by Delete', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the history is counted through the hook of the dev server')
  // Zoomed out, so that the three bars used are in sight.
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  await clipAt(page, 0, 0)
  await clipAt(page, 0, 2)
  // Track 2 stays empty: the arrows go past it.
  await clipAt(page, 2, 1)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '3')
  const cells = page.getByTestId('arr-item')
  await expect(cells).toHaveCount(3)
  await expect(page.getByTestId('arr-items')).toHaveAttribute('role', 'grid')
  await expect(cells.first()).toHaveAccessibleName(/^Synth \d+, Synth, bars 1 to 1/)

  // One cell in the tab order: Tab from the timeline lands on it.
  await page.getByTestId('arr-stage').getByTestId('timegrid').focus()
  await page.keyboard.press('Tab')
  const focused = page.locator('[data-testid=arr-item]:focus')
  await expect(focused).toHaveCount(1)
  await page.keyboard.press('Escape')

  // Right goes to the next clip of the track, selected; Down to the nearest clip of the next track that holds one (Track 2 is empty).
  await cells.first().focus()
  await page.keyboard.press('ArrowRight')
  await expect(focused).toHaveAccessibleName(/Synth, bars 3 to 3, selected$/)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '1')
  await page.keyboard.press('ArrowDown')
  await expect(focused).toHaveAccessibleName(/Track 3, bars 2 to 2, selected$/)
  // Shift+Up extends the selection.
  await page.keyboard.press('Shift+ArrowUp')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '2')
  mkdirSync(SHOTS, { recursive: true })
  await page.screenshot({ path: `${SHOTS}/timeline-keyboard-focus.png` })

  // Alt+Up has no command (a clip stays on its track): it says so, and again on a second press (the text changes, so it is read again).
  await page.keyboard.press('Alt+ArrowUp')
  const said = page.getByTestId('arr-said')
  await expect(said).toHaveText(/stays on its track/)
  const first = await said.evaluate((e) => e.textContent)
  await page.keyboard.press('Alt+ArrowUp')
  await expect.poll(() => said.evaluate((e) => e.textContent)).not.toBe(first)
  await expect(said).toHaveText(/stays on its track/)

  // Ctrl+= and Ctrl+- on a cell zoom the timeline, not the page, and the focus stays on the clip.
  const grid = page.getByTestId('arr-stage').getByTestId('timegrid')
  const zoom = async () => Number(await grid.getAttribute('data-px-per-tick'))
  const z0 = await zoom()
  const name = (await focused.getAttribute('aria-label'))!
  await page.keyboard.press('Control+Equal')
  await expect.poll(zoom).toBeGreaterThan(z0)
  await page.keyboard.press('Control+Minus')
  await expect.poll(zoom).toBeCloseTo(z0, 6)
  expect(await page.evaluate(() => Math.round(window.devicePixelRatio * 100))).toBe(100)
  await expect(focused).toHaveAccessibleName(name)

  // Alt+Right moves the selected clips by the snap step, as one command (one history entry), and says so.
  const before = await blocks(page)
  const entries = await done(page)
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start)).not.toEqual(before.map((b) => b.start))
  expect(await done(page)).toBe(entries + 1)
  await expect(page.getByTestId('arr-said')).toHaveText(/Moved to bar \d+/)
  await expect(page.getByTestId('arr-said')).toHaveAttribute('aria-live', 'polite')
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start)).toEqual(before.map((b) => b.start))

  // A move onto another clip carves it, and the live region says so: with the snap on bars, the first Synth clip moved twice
  // lands on the second.
  await page.getByTestId('arr-snap').selectOption('bar')
  await cells.first().focus()
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('Alt+ArrowRight')
  await page.keyboard.press('Alt+ArrowRight')
  await expect(page.getByTestId('arr-said')).toHaveText(/Clips cut under it: 1/)
  await page.keyboard.press('Control+z')
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start)).toEqual(before.map((b) => b.start))

  // Delete removes the selection (two clips here) as one command and says so; the focus goes to the clip that stays, on another
  // track (the Synth row is empty then), never to a deleted one.
  await cells.first().focus()
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('Shift+ArrowRight')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '2')
  const beforeDelete = await done(page)
  await page.keyboard.press('Delete')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '1')
  await expect(page.getByTestId('arr-said')).toHaveText(/deleted: 2/i)
  expect(await done(page)).toBe(beforeDelete + 1)
  await expect(focused).toHaveCount(1)
  await expect(focused).toHaveAccessibleName(/Track 3/)

  // Enter opens the clip in the piano roll.
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  expect(errors).toEqual([])
})

test('the notes are named cells: arrows move between them, Alt+arrows move and transpose, Delete removes', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('arr-track').first().click()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  const roll = page.getByTestId('piano-roll')
  await expect(roll).toHaveAttribute('data-note-count', '0')
  const clip = await roll.getAttribute('data-clip')
  await run(page, {
    type: 'clip.notes.add',
    clip,
    notes: [
      { start: 0, pitch: 60, length: 960, velocity: 100 },
      { start: 960, pitch: 64, length: 960, velocity: 90 },
      { start: 960, pitch: 67, length: 480, velocity: 80 },
    ],
  })
  await expect(roll).toHaveAttribute('data-note-count', '3')
  const cells = page.getByTestId('roll-item')
  await expect(cells).toHaveCount(3)
  const focused = page.locator('[data-testid=roll-item]:focus')
  await cells.first().focus()
  await expect(focused).toHaveAccessibleName(/^C4, bar 1 beat 1, 1 beat, velocity 100/)
  // Right: the next note in time (the chord's top first); Down: the nearest lower note.
  await page.keyboard.press('ArrowRight')
  await expect(focused).toHaveAccessibleName(/^G4, bar 1 beat 2, 0.5 beats, velocity 80, selected$/)
  await page.keyboard.press('ArrowDown')
  await expect(focused).toHaveAccessibleName(/^E4, bar 1 beat 2/)
  // Alt+Up transposes it by a semitone, as one command (one history entry); Alt+Right moves it by the snap step.
  const entries = await done(page)
  await page.keyboard.press('Alt+ArrowUp')
  await expect(focused).toHaveAccessibleName(/^F4, bar 1 beat 2/)
  expect(await done(page)).toBe(entries + 1)
  await expect(page.getByTestId('roll-said')).toHaveText(/F4/)
  await page.keyboard.press('Alt+ArrowRight')
  await expect(focused).toHaveAccessibleName(/^F4, bar 1 beat 2\.25/)
  expect(await done(page)).toBe(entries + 2)
  // Ctrl+= on a note zooms the roll, not the page, and the focus stays.
  const grid = page.getByTestId('roll-stage').getByTestId('timegrid')
  const z0 = Number(await grid.getAttribute('data-px-per-tick'))
  await page.keyboard.press('Control+Equal')
  await expect.poll(async () => Number(await grid.getAttribute('data-px-per-tick'))).toBeGreaterThan(z0)
  await expect(focused).toHaveAccessibleName(/^F4/)
  await page.keyboard.press('Control+z')
  await page.keyboard.press('Control+z')
  await expect(roll).toHaveAttribute('data-note-count', '3')
  // Delete removes the first note as one command; the focus goes to the next note in time.
  await cells.first().focus()
  await expect(focused).toHaveAccessibleName(/^C4/)
  const beforeDelete = await done(page)
  await page.keyboard.press('Delete')
  await expect(roll).toHaveAttribute('data-note-count', '2')
  await expect(page.getByTestId('roll-said')).toHaveText(/deleted/i)
  expect(await done(page)).toBe(beforeDelete + 1)
  await expect(focused).toHaveCount(1)
  await expect(focused).toHaveAccessibleName(/^G4, bar 1 beat 2/)
  await page.screenshot({ path: `${SHOTS}/piano-roll-keyboard-focus.png` })
  expect(errors).toEqual([])
})

test('after Ctrl+D the copies are the selection; Shift+arrow from the original starts a new one; Ctrl+D copies the focused clip (#528)', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  await clipAt(page, 0, 0)
  await clipAt(page, 0, 1)
  const arr = page.getByTestId('arrangement')
  await expect(arr).toHaveAttribute('data-block-count', '2')
  const cell = (id: string) => page.locator(`[data-testid=arr-item][data-block="${id}"]`)
  const [a, b] = (await blocks(page)).sort((x, y) => x.start - y.start)

  // Two clips selected from the keyboard: the Duplicate button says how many before the copy.
  await cell(a!.id).focus()
  await page.keyboard.press('Shift+ArrowRight')
  await expect(arr).toHaveAttribute('data-selected-count', '2')
  await expect(page.getByTestId('arr-duplicate')).toContainText('(2)')
  await page.keyboard.press('Control+d')
  await expect(arr).toHaveAttribute('data-block-count', '4')
  await expect(arr).toHaveAttribute('data-selected-count', '2')
  const copies = JSON.parse((await arr.getAttribute('data-selection')) ?? '[]') as string[]
  expect(copies).not.toContain(a!.id)

  // From the original, Shift+arrow starts a new selection of two clips: the copies are not added to it.
  await cell(a!.id).focus()
  await page.keyboard.press('Shift+ArrowRight')
  await expect(arr).toHaveAttribute('data-selected-count', '2')
  const fresh = JSON.parse((await arr.getAttribute('data-selection')) ?? '[]') as string[]
  expect([...fresh].sort()).toEqual([a!.id, b!.id].sort())
  // The two copies of the first Ctrl+D are untouched; this one lands on them (a copy carves what it covers), never more than two clips.
  await page.keyboard.press('Control+d')
  await expect(arr).toHaveAttribute('data-selected-count', '2')
  expect(Number(await arr.getAttribute('data-block-count'))).toBeLessThanOrEqual(6)

  // Ctrl+D acts on the focused clip when the selection does not hold it.
  await cell(b!.id).focus()
  await page.keyboard.press('ArrowRight')
  const lone = JSON.parse((await arr.getAttribute('data-selection')) ?? '[]') as string[]
  expect(lone).toHaveLength(1)
  const count = Number(await arr.getAttribute('data-block-count'))
  const before = new Set((await blocks(page)).map((x) => x.id))
  await cell(a!.id).focus()
  await page.keyboard.press('Control+d')
  await expect.poll(async () => (await blocks(page)).filter((x) => !before.has(x.id)).length).toBeGreaterThan(0)
  expect(Number(await arr.getAttribute('data-selected-count'))).toBe(1)
  expect(count).toBeGreaterThan(0)
  expect(errors).toEqual([])
})
