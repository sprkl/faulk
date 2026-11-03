import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #494: writing from the keyboard. The timeline and the piano roll each have a free cursor (the only tab stop of an empty
// grid); Enter on it creates a clip of one bar or a note of the drawing length, Escape on an item frees it there, Page Up and
// Page Down set the selection's velocity, Enter on a placement opens the piano roll on that placement, and undo and redo
// are said in the live regions. Real worklet, engine and project core: every edit is a command, read back from the project.

test.use({ storageState: { cookies: [], origins: [] } })

const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
/** How many entries the history holds (done, not undone): one key, one entry. */
const done = (page: Page) => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[] } }).__faulkHistory().undo.length)
const blocks = async (page: Page): Promise<{ id: string; row: number; start: number; clip: string }[]> =>
  JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')
const rollNotes = async (page: Page): Promise<{ start: number; pitch: number; velocity: number }[]> =>
  JSON.parse((await page.getByTestId('piano-roll').getAttribute('data-notes')) ?? '[]')

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

test('the piano roll: a note from the cursor of an empty clip, its velocity by Page Up and Down, undo and redo said', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the history is counted through the hook of the dev server')
  await page.getByTestId('arr-track').first().click()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  const roll = page.getByTestId('piano-roll')
  await expect(roll).toHaveAttribute('data-note-count', '0')
  await page.getByTestId('roll-snap').selectOption('quarter-beat')

  // An empty clip: Tab from the grid lands on the cursor, its only cell.
  await page.getByTestId('roll-stage').getByTestId('timegrid').focus()
  await page.keyboard.press('Tab')
  const cursor = page.getByTestId('roll-cursor')
  await expect(cursor).toBeFocused()
  await expect(cursor).toHaveAccessibleName(/^Cursor, [A-G]#?-?\d, bar 1 beat 1$/)
  const pitch = Number(await cursor.getAttribute('data-pitch'))
  // Right by the drawing step (a sixteenth), Up by a semitone.
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/bar 1 beat 1\.25$/)
  await page.keyboard.press('ArrowUp')
  await expect(cursor).toHaveAttribute('data-pitch', String(pitch + 1))

  // Enter writes a note there (one command), and the focus goes to it.
  const before = await done(page)
  await page.keyboard.press('Enter')
  await expect(roll).toHaveAttribute('data-note-count', '1')
  expect(await done(page)).toBe(before + 1)
  const focused = page.locator('[data-testid=roll-item]:focus')
  await expect(focused).toHaveAccessibleName(/bar 1 beat 1\.25, .*velocity 100, selected$/)
  await expect(page.getByTestId('roll-said')).toHaveText(/^Note added/)
  expect(await rollNotes(page)).toEqual([expect.objectContaining({ start: 240, pitch: pitch + 1, velocity: 100 })])

  // Page Up: +10, Shift+Page Down: -1, each one `clip.notes.update`, said; kept at 127.
  await page.keyboard.press('PageUp')
  await expect(page.getByTestId('roll-said')).toHaveText(/^Velocity 110/)
  await page.keyboard.press('Shift+PageDown')
  await expect(page.getByTestId('roll-said')).toHaveText(/^Velocity 109/)
  expect(await done(page)).toBe(before + 3)
  await expect(focused).toHaveAccessibleName(/velocity 109, selected$/)
  for (let i = 0; i < 2; i++) await page.keyboard.press('PageUp')
  await expect(page.getByTestId('roll-said')).toHaveText(/^Velocity 127/)
  await page.keyboard.press('PageUp')
  await expect(page.getByTestId('roll-said')).toHaveText(/^Velocity already at 127/)
  expect((await rollNotes(page))[0]!.velocity).toBe(127)

  // Undo and redo say what they did.
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('roll-said')).toHaveText(/^Undone: Edit notes/)
  expect((await rollNotes(page))[0]!.velocity).toBe(119)
  // The note focused changed under the undo: the focus stays in the grid.
  await expect(focused).toHaveCount(1)
  await page.keyboard.press('Control+Shift+z')
  await expect(page.getByTestId('roll-said')).toHaveText(/^Redone: Edit notes/)
  expect((await rollNotes(page))[0]!.velocity).toBe(127)

  // Escape on a note frees the cursor where it is; Enter there goes to the note rather than writing over it.
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Escape')
  await expect(cursor).toBeFocused()
  await expect(cursor).toHaveAccessibleName(/bar 1 beat 1\.25$/)
  await expect(roll).toHaveAttribute('data-selected-count', '1')
  await page.keyboard.press('Enter')
  await expect(focused).toHaveCount(1)
  await expect(roll).toHaveAttribute('data-note-count', '1')
  // Escape, then Escape again on the cursor clears the selection; Down and Enter write a second note.
  await page.keyboard.press('Escape')
  await page.keyboard.press('Escape')
  await expect(roll).toHaveAttribute('data-selected-count', '0')
  await expect(cursor).toBeFocused()
  // Page Up with nothing selected says so rather than nothing.
  await page.keyboard.press('PageUp')
  await expect(page.getByTestId('roll-said')).toHaveText(/^No note selected/)
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(roll).toHaveAttribute('data-note-count', '2')
  // The velocity of a new note is the last one set.
  expect((await rollNotes(page)).map((n) => n.velocity)).toEqual([127, 127])
  // Delete the last notes from the keyboard: the focus falls back on the cursor, never on nothing.
  await page.keyboard.press('Delete')
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Delete')
  await expect(roll).toHaveAttribute('data-note-count', '0')
  await expect(cursor).toBeFocused()
  expect(errors).toEqual([])
})

test('the timeline: a clip from the cursor, and Enter opens the piano roll on the placement activated', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the history is counted through the hook of the dev server')
  // No clip: Tab from the timeline lands on the cursor, named after its track and bar.
  await page.getByTestId('arr-stage').getByTestId('timegrid').focus()
  await page.keyboard.press('Tab')
  const cursor = page.getByTestId('arr-cursor')
  await expect(cursor).toBeFocused()
  await expect(cursor).toHaveAccessibleName(/^Cursor, .+, bar 1$/)
  // Up to the first track. With the snap on Auto (a 1/32 at the default zoom), Right still goes by a beat at least.
  await page.keyboard.press('ArrowUp')
  await page.keyboard.press('ArrowUp')
  await expect(cursor).toHaveAttribute('data-row', '0')
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/^Cursor, Synth, bar 1 beat 2$/)
  await page.keyboard.press('ArrowLeft')
  // With the snap on bars, Right goes by a bar.
  await page.getByTestId('arr-snap').selectOption('bar')
  await cursor.focus()
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/^Cursor, Synth, bar 2$/)

  // Enter creates a clip of one bar there (one command), focused and said.
  const before = await done(page)
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '1')
  expect(await done(page)).toBe(before + 1)
  const focused = page.locator('[data-testid=arr-item]:focus')
  await expect(focused).toHaveAccessibleName(/, Synth, bars 2 to 2, selected$/)
  await expect(page.getByTestId('arr-said')).toHaveText(/^Clip created, bar 2/)

  // Undo is said here too, and redo brings the clip back.
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('arr-said')).toHaveText(/^Undone: Create a clip/)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '0')
  // The clip focused is gone: the focus goes back to the grid's tab stop, the cursor.
  await expect(cursor).toBeFocused()
  await page.keyboard.press('Control+y')
  await expect(page.getByTestId('arr-said')).toHaveText(/^Redone: Create a clip/)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '1')

  // A linked copy right after it (Ctrl+Shift+D): two placements of one clip. Right goes to the copy, Enter opens it.
  await page.getByTestId('arr-item').first().focus()
  await page.keyboard.press('Control+Shift+d')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '2')
  const placed = await blocks(page)
  expect(new Set(placed.map((b) => b.clip)).size).toBe(1)
  await page.getByTestId('arr-item').first().focus()
  await page.keyboard.press('ArrowRight')
  await expect(focused).toHaveAccessibleName(/bars 3 to 3, selected$/)
  const copy = placed.find((b) => b.start === Math.max(...placed.map((x) => x.start)))!
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  await expect(page.getByTestId('roll-clip-origin')).toHaveText('Arrangement · bar 3')
  await expect(page.getByTestId('roll-clip')).toHaveValue(`placement:${copy.id}`)
  await expect(page.getByTestId('roll-clip').locator('option')).toHaveCount(2)

  // Escape on a clip frees the cursor at its start; Left (a bar with the snap on bars) and Enter create a clip at bar 1.
  await page.getByTestId('arr-item').first().focus()
  await page.keyboard.press('Escape')
  await expect(cursor).toBeFocused()
  await expect(cursor).toHaveAccessibleName(/^Cursor, Synth, bar 2$/)
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('Shift+ArrowLeft')
  await expect(cursor).toHaveAccessibleName(/^Cursor, Synth, bar 1$/)
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '3')

  // Enter on the cursor over a clip goes to that clip and creates nothing.
  await page.keyboard.press('Escape')
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/^Cursor, Synth, bar 2$/)
  await page.keyboard.press('Enter')
  await expect(focused).toHaveAccessibleName(/, Synth, bars 2 to 2, selected$/)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '3')

  // No room for a bar before the next clip: refused in words. A clip at bar 5, then the cursor at bar 4 beat 2.
  await page.keyboard.press('Escape')
  for (let i = 0; i < 3; i++) await page.keyboard.press('Shift+ArrowRight')
  await expect(cursor).toHaveAccessibleName(/^Cursor, Synth, bar 5$/)
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '4')
  await page.keyboard.press('Escape')
  await page.keyboard.press('Shift+ArrowLeft')
  await page.getByTestId('arr-snap').selectOption('beat')
  await cursor.focus()
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/^Cursor, Synth, bar 4 beat 2$/)
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('arr-said')).toHaveText(/^No room for a one-bar clip here/)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '4')
  expect(errors).toEqual([])
})
