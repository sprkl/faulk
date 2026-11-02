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
const blocks = async (page: Page): Promise<{ id: string; row: number; start: number }[]> =>
  JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')

async function start(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  await page.getByTestId('add-track').click()
  await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Drums', exact: true }).click()
  await expect(page.getByTestId('track-header')).toHaveCount(2)
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
  // Zoomed out, so that the three bars used are in sight.
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  await clipAt(page, 0, 0)
  await clipAt(page, 0, 2)
  await clipAt(page, 1, 1)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '3')
  const cells = page.getByTestId('arr-item')
  await expect(cells).toHaveCount(3)
  await expect(page.getByTestId('arr-items')).toHaveAttribute('role', 'grid')
  await expect(cells.first()).toHaveAccessibleName(/^Clip( \d+)?, Synth, bars 1 to 1/)

  // One cell in the tab order: Tab from the timeline lands on it.
  await page.getByTestId('arr-stage').getByTestId('timegrid').focus()
  await page.keyboard.press('Tab')
  const focused = page.locator('[data-testid=arr-item]:focus')
  await expect(focused).toHaveCount(1)
  await page.keyboard.press('Escape')

  // Right goes to the next clip of the track, selected; Down to the nearest clip of the next track.
  await cells.first().focus()
  await page.keyboard.press('ArrowRight')
  await expect(focused).toHaveAccessibleName(/Synth, bars 3 to 3, selected$/)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '1')
  await page.keyboard.press('ArrowDown')
  await expect(focused).toHaveAccessibleName(/Track 2, bars 2 to 2, selected$/)
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

  // Alt+Right moves the selected clips by the snap step (a bar at this zoom), as one command, and says so.
  const before = await blocks(page)
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start)).not.toEqual(before.map((b) => b.start))
  await expect(page.getByTestId('arr-said')).toHaveText(/Moved to bar \d+/)
  await expect(page.getByTestId('arr-said')).toHaveAttribute('aria-live', 'polite')
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start)).toEqual(before.map((b) => b.start))

  // Delete removes the selection (two clips here) and says so; the focus goes to the clip that stays, not to a deleted one.
  await cells.first().focus()
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('Shift+ArrowRight')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '2')
  await page.keyboard.press('Delete')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '1')
  await expect(page.getByTestId('arr-said')).toHaveText(/deleted: 2/i)
  await expect(focused).toHaveCount(1)
  await expect(focused).toHaveAccessibleName(/Track 2/)

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
  // Alt+Up transposes it by a semitone, as one command; Alt+Right moves it by the snap step.
  await page.keyboard.press('Alt+ArrowUp')
  await expect(focused).toHaveAccessibleName(/^F4, bar 1 beat 2/)
  await expect(page.getByTestId('roll-said')).toHaveText(/F4/)
  await page.keyboard.press('Alt+ArrowRight')
  await expect(focused).toHaveAccessibleName(/^F4, bar 1 beat 2\.25/)
  await page.keyboard.press('Control+z')
  await page.keyboard.press('Control+z')
  // Delete removes it.
  await page
    .getByTestId('roll-item')
    .filter({ has: page.locator(':scope') })
    .first()
    .focus()
  await page.keyboard.press('Delete')
  await expect(roll).toHaveAttribute('data-note-count', '2')
  await expect(page.getByTestId('roll-said')).toHaveText(/deleted/i)
  await page.screenshot({ path: `${SHOTS}/piano-roll-keyboard-focus.png` })
  expect(errors).toEqual([])
})
