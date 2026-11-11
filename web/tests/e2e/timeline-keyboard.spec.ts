import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

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
const blocks = async (page: Page): Promise<{ id: string; row: number; start: number; clip: string }[]> =>
  JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')

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

  // Alt+Up on a selection whose first clip is on the first track goes nowhere (all or none): it says so, and again on a second press
  // (the text changes, so it is read again). Nothing changes in the project.
  const atRest = await done(page)
  await page.keyboard.press('Alt+ArrowUp')
  const said = page.getByTestId('arr-said')
  await expect(said).toHaveText(/No track there/)
  const first = await said.evaluate((e) => e.textContent)
  await page.keyboard.press('Alt+ArrowUp')
  await expect.poll(() => said.evaluate((e) => e.textContent)).not.toBe(first)
  await expect(said).toHaveText(/No track there/)
  expect(await done(page)).toBe(atRest)

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
  // Right: the next note in time (the chord's top first), the selection unchanged (#771); Down: the nearest lower note.
  await page.keyboard.press('ArrowRight')
  await expect(focused).toHaveAccessibleName(/^G4, bar 1 beat 2, 0.5 beats, velocity 80$/)
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

test('after Ctrl+D the copies are the selection and the focus; Shift+arrow from the original starts a new selection; Ctrl+D copies a focused clip outside the selection (#528)', async ({
  page,
}) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  await clipAt(page, 0, 0)
  await clipAt(page, 0, 1)
  const arr = page.getByTestId('arrangement')
  await expect(arr).toHaveAttribute('data-block-count', '2')
  const cell = (id: string) => page.locator(`[data-testid=arr-item][data-block="${id}"]`)
  const selection = async () => (JSON.parse((await arr.getAttribute('data-selection')) ?? '[]') as string[]).sort()
  const [a, b] = (await blocks(page)).sort((x, y) => x.start - y.start)

  // Two clips selected from the keyboard: the Duplicate button says how many before the copy.
  await cell(a!.id).focus()
  await page.keyboard.press('Shift+ArrowRight')
  await expect(arr).toHaveAttribute('data-selected-count', '2')
  await expect(page.getByTestId('arr-duplicate')).toContainText('(2)')
  await page.keyboard.press('Control+d')
  await expect(arr).toHaveAttribute('data-block-count', '4')
  await expect(page.locator('[data-testid=arr-item]:focus')).toHaveCount(1)
  await expect(page.locator('[data-testid=arr-item]:focus')).not.toHaveAttribute('data-block', a!.id)
  const copies = await selection()
  expect(copies).toHaveLength(2)
  expect(copies).not.toContain(a!.id)
  expect(copies).not.toContain(b!.id)

  // The focus follows the copies: a second Ctrl+D copies them (two more clips, six in all), not the original.
  await page.keyboard.press('Control+d')
  await expect(arr).toHaveAttribute('data-block-count', '6')
  const second = await selection()
  expect(second).toHaveLength(2)
  for (const id of [a!.id, b!.id, ...copies]) expect(second).not.toContain(id)

  // From the original, Shift+arrow starts a new selection of two clips: the copies are not added to it.
  await cell(a!.id).focus()
  await page.keyboard.press('Shift+ArrowRight')
  expect(await selection()).toEqual([a!.id, b!.id].sort())
  await expect(arr).toHaveAttribute('data-selected-count', '2')

  // With nothing selected (a click on an empty row clears it), Ctrl+D on the keyboard-focused clip is refused when the clip after it
  // is there (#805): the first track holds six bars of clips.
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const clear = () => page.mouse.click(box.x + 100 * ppt, box.y + RULER + 2 * LANE + LANE / 2)
  await clear()
  await expect(arr).toHaveAttribute('data-selected-count', '0')
  await cell(a!.id).focus()
  await page.keyboard.press('Control+d')
  await expect(page.getByTestId('arr-said')).toHaveText(/Not copied/)
  await expect(arr).toHaveAttribute('data-block-count', '6')

  // On the second track, where the place after is free, it copies the focused clip.
  await clipAt(page, 1, 0)
  const c = (await blocks(page)).find((x) => x.row === 1)!
  await clear()
  await expect(arr).toHaveAttribute('data-selected-count', '0')
  await cell(c.id).focus()
  await page.keyboard.press('Control+d')
  const after = await selection()
  expect(after).toHaveLength(1)
  expect(after).not.toContain(c.id)
  // The copy is of the focused clip: it sits one clip length after it (the clip is a bar long).
  const copy = (await blocks(page)).find((x) => x.id === after[0])
  expect(copy?.row).toBe(c.row)
  expect(copy?.start).toBe(c.start + 3840)
  // A pointer click on a clip moves the keyboard focus off the cell that held it: Ctrl+D then copies the clicked clip, not the cell.
  await cell(c.id).focus()
  await page.mouse.click(box.x + (copy!.start + 1920) * ppt, box.y + RULER + LANE + LANE / 2)
  const clicked = await selection()
  expect(clicked).toEqual([copy!.id])
  await page.keyboard.press('Control+d')
  const viaPointer = await selection()
  expect(viaPointer).toHaveLength(1)
  expect(viaPointer).not.toContain(copy!.id)
  const fromPointer = (await blocks(page)).find((x) => x.id === viaPointer[0])
  expect(fromPointer?.start).toBe(copy!.start + 3840)
  expect(errors).toEqual([])
})

test('a clip moves to the track below and back from the keyboard, the menu and the mouse, each as one history entry', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the history is counted through the hook of the dev server')
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  await clipAt(page, 0, 0)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '1')
  const cells = page.getByTestId('arr-item')
  const focused = page.locator('[data-testid=arr-item]:focus')
  const rowOf = async () => (await blocks(page)).map((b) => b.row)
  expect(await rowOf()).toEqual([0])

  // Alt+Down: the clip goes to the track below (a copy of the clip comes with it), as one entry, and says so; the focus follows it.
  await cells.first().focus()
  let entries = await done(page)
  await page.keyboard.press('Alt+ArrowDown')
  await expect.poll(rowOf).toEqual([1])
  expect(await done(page)).toBe(entries + 1)
  await expect(page.getByTestId('arr-said')).toHaveText(/Moved to .*, bar 1/)
  await expect(focused).toHaveCount(1)
  // One undo brings it back.
  await page.keyboard.press('Control+z')
  await expect.poll(rowOf).toEqual([0])

  // The right-click menu: the entry above is disabled on the first track, the one below moves the clip.
  const area = page.getByTestId('arr-stage')
  const at = (await area.boundingBox())!
  const scale = Number(await area.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.click(at.x + (3840 / 2) * scale, at.y + RULER + LANE / 2, { button: 'right' })
  const menu = page.getByRole('menu')
  await expect(menu.getByRole('menuitem', { name: /Move to the track above/ })).toBeDisabled()
  entries = await done(page)
  await menu.getByRole('menuitem', { name: /Move to the track below/ }).click()
  await expect.poll(rowOf).toEqual([1])
  expect(await done(page)).toBe(entries + 1)

  // The mouse: dragging the clip up to the first row moves it there (one command), and in time with it.
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const x = box.x + (3840 / 2) * ppt
  entries = await done(page)
  await page.mouse.move(x, box.y + RULER + LANE + LANE / 2)
  await page.mouse.down()
  await page.mouse.move(x + 20, box.y + RULER + LANE / 2 + 12, { steps: 6 })
  await page.mouse.move(x + 40, box.y + RULER + LANE / 2, { steps: 6 })
  await page.mouse.up()
  await expect.poll(rowOf).toEqual([0])
  expect(await done(page)).toBe(entries + 1)

  // Down past the last track is refused out loud and changes nothing.
  await cells.first().focus()
  await page.keyboard.press('Alt+ArrowDown')
  await page.keyboard.press('Alt+ArrowDown')
  await expect.poll(rowOf).toEqual([2])
  entries = await done(page)
  await page.keyboard.press('Alt+ArrowDown')
  await expect(page.getByTestId('arr-said')).toHaveText(/No track there/)
  expect(await done(page)).toBe(entries)

  // An audio track below is not a target: refused out loud, nothing moves and no entry is made.
  await run(page, { type: 'track.create', kind: 'audio' })
  await expect(page.getByTestId('track-header')).toHaveCount(4)
  entries = await done(page)
  await cells.first().focus()
  await page.keyboard.press('Alt+ArrowDown')
  await expect(page.getByTestId('arr-said')).toHaveText(/is not a MIDI track/)
  expect(await done(page)).toBe(entries)
  expect(await rowOf()).toEqual([2])
  expect(errors).toEqual([])
})

test('the free cursor reaches the ruler row with Up; Enter there and Shift+Enter on a track put the playhead at the cursor (#554)', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const arr = page.getByTestId('arrangement')
  await expect(arr).toHaveAttribute('data-block-count', '0')
  const cursor = page.getByTestId('arr-cursor')
  const position = page.getByTestId('transport-position')
  await cursor.focus()
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  const tick = Number(await cursor.getAttribute('data-tick'))
  expect(tick).toBeGreaterThan(0)
  const at = `${Math.floor(tick / 3840) + 1}.${Math.floor((tick % 3840) / 960) + 1}.`

  // Shift+Enter on a track row: the playhead goes there, and no clip is created.
  await page.keyboard.press('Shift+Enter')
  await expect(position).toContainText(at)
  await expect(page.getByTestId('arr-said')).toHaveText(/^Playhead at bar/)
  await expect(arr).toHaveAttribute('data-block-count', '0')

  // Up from the first track reaches the ruler row; Right moves along it, Enter puts the playhead on the new tick.
  for (let i = 0; i < 4; i++) await page.keyboard.press('ArrowUp')
  await expect(cursor).toHaveAttribute('data-row', '-1')
  await expect(cursor).toBeFocused()
  await expect(cursor).toHaveAccessibleName(/^Ruler, bar/)
  await page.keyboard.press('ArrowRight')
  const later = Number(await cursor.getAttribute('data-tick'))
  expect(later).toBeGreaterThan(tick)
  await page.keyboard.press('Enter')
  await expect(position).toContainText(`${Math.floor(later / 3840) + 1}.${Math.floor((later % 3840) / 960) + 1}.`)
  await expect(arr).toHaveAttribute('data-block-count', '0')

  // Down comes back to the first track.
  await page.keyboard.press('ArrowDown')
  await expect(cursor).toHaveAttribute('data-row', '0')

  // Cut at a bar the keyboard reached: a clip of one bar, the playhead put at its beat 3 by Ctrl+G, then Ctrl+E cuts it in two.
  await clipAt(page, 0, 2)
  await expect(arr).toHaveAttribute('data-block-count', '1')
  const [clip] = await blocks(page)
  await page.locator(`[data-testid=arr-item][data-block="${clip!.id}"]`).focus()
  await page.keyboard.press('Shift+ArrowRight')
  await expect(arr).toHaveAttribute('data-selected-count', '1')
  await page.keyboard.press('Control+g')
  await page.keyboard.type('3.3')
  await page.keyboard.press('Enter')
  await expect(position).toContainText('3.3')
  await page.keyboard.press('Control+e')
  await expect(arr).toHaveAttribute('data-block-count', '2')
  expect(errors).toEqual([])
})

test('a clip of 8 bars is repeated up to bar 64 as linked placements, from the keyboard and the menu, one undo each (#555)', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the clip is resized and the history counted through the hooks of the dev server')
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  await clipAt(page, 0, 0)
  const arr = page.getByTestId('arrangement')
  await expect(arr).toHaveAttribute('data-block-count', '1')
  const [only] = await blocks(page)
  await run(page, { type: 'arrangement.resize', placement: only!.id, edge: 'end', to: 3840 * 8 })
  const cell = page.locator(`[data-testid=arr-item][data-block="${only!.id}"]`)
  const field = page.getByTestId('repeat-field')

  // Ctrl+Shift+R on the focused clip: the field opens (the page does not reload), a bar before the clip's end is refused.
  await clearSelection(page)
  await cell.focus()
  await page.keyboard.press('Control+Shift+r')
  await expect(page.getByTestId('repeat-dialog')).toBeVisible()
  await expect(field).toHaveValue('9')
  // The chord is cancelled even with the field open and typed in (the browser would reload the page), and does not open a second one.
  const cancelled = await page.evaluate(() => {
    const event = new KeyboardEvent('keydown', { key: 'R', ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true })
    document.activeElement?.dispatchEvent(event)
    return event.defaultPrevented
  })
  expect(cancelled).toBe(true)
  await expect(page.getByTestId('repeat-dialog')).toBeVisible()
  await field.fill('8')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('repeat-error')).toBeVisible()
  const entries = await done(page)
  await field.fill('64')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('repeat-dialog')).toBeHidden()
  await expect(arr).toHaveAttribute('data-block-count', '8')
  expect(new Set((await blocks(page)).map((b) => b.clip)).size).toBe(1)
  expect(await done(page)).toBe(entries + 1)

  // The Clips tab (of the clip's track, which a selection makes the selected track) shows one linked clip with its eight placements.
  await cell.focus()
  await page.keyboard.press('Shift+ArrowRight')
  await page.getByTestId('panel-clips').click()
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  await expect(page.getByTestId('pool-clip')).toContainText('8 in the arrangement')
  await expect(page.getByTestId('pool-clip')).toContainText('Linked')

  // One undo takes every copy away; with nothing selected or focused the shortcut says so, with one clip selected it opens the field.
  await page.keyboard.press('Control+z')
  await expect(arr).toHaveAttribute('data-block-count', '1')
  await clearSelection(page)
  await page.keyboard.press('Control+Shift+r')
  await expect(page.getByTestId('arr-said')).toHaveText(/Select one clip/)
  await expect(page.getByTestId('repeat-dialog')).toBeHidden()
  await cell.focus()
  await page.keyboard.press('Shift+ArrowRight')
  await page.keyboard.press('Control+Shift+r')
  await expect(page.getByTestId('repeat-dialog')).toBeVisible()
  await field.fill('24')
  await page.keyboard.press('Enter')
  await expect(arr).toHaveAttribute('data-block-count', '3')

  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.click(box.x + 3840 * 4 * ppt, box.y + RULER + LANE / 2, { button: 'right' })
  await page.getByRole('menuitem', { name: /Repeat until/ }).click()
  await field.fill('40')
  await page.keyboard.press('Enter')
  await expect(arr).toHaveAttribute('data-block-count', '5')
  expect(errors).toEqual([])
})

/** A click on an empty row of the third track clears the selection. */
async function clearSelection(page: Page) {
  const box = (await page.getByTestId('arr-stage').boundingBox())!
  await page.mouse.click(box.x + 600, box.y + RULER + 2 * LANE + LANE / 2)
}

test('a lone clip is selected from the keyboard by Shift+arrow, then Ctrl+D copies it (#553)', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await clipAt(page, 0, 0)
  const arr = page.getByTestId('arrangement')
  await expect(arr).toHaveAttribute('data-block-count', '1')
  const [only] = await blocks(page)
  const cell = page.locator(`[data-testid=arr-item][data-block="${only!.id}"]`)

  // Focused by the keyboard, the clip is not selected; a Shift+arrow with no neighbour selects it.
  await clearSelection(page)
  await expect(arr).toHaveAttribute('data-selected-count', '0')
  await cell.focus()
  await expect(arr).toHaveAttribute('data-selected-count', '0')
  await page.keyboard.press('Shift+ArrowRight')
  await expect(arr).toHaveAttribute('data-selected-count', '1')
  await expect(page.getByTestId('arr-said')).toHaveText(/selected/)
  await page.keyboard.press('Control+d')
  await expect(arr).toHaveAttribute('data-block-count', '2')

  // Focused and not selected: Alt+arrow moves it and Delete removes it, without any selection step.
  const [first] = (await blocks(page)).sort((x, y) => x.start - y.start)
  await clearSelection(page)
  await expect(arr).toHaveAttribute('data-selected-count', '0')
  await page.locator(`[data-testid=arr-item][data-block="${first!.id}"]`).focus()
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(async () => (await blocks(page)).find((x) => x.id === first!.id)?.start).toBeGreaterThan(first!.start)
  await page.keyboard.press('Delete')
  await expect(arr).toHaveAttribute('data-block-count', '1')
  expect(errors).toEqual([])
})
