import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #882: step entry in the piano roll. A mode (toolbar button, Alt+E) in which the musical-typing letters write notes at the
// free cursor: a phrase, a chord (letters held together, the cursor advances when the last is released), a rest (Period),
// a longer last step (Comma), Backspace; each note is one `clip.notes.add`, so one undo each. Real worklet and engine.

interface Note {
  start: number
  pitch: number
  length: number
  velocity: number
}

const roll = (page: Page) => page.getByTestId('piano-roll')
const notes = async (page: Page): Promise<Note[]> => JSON.parse((await roll(page).getAttribute('data-notes')) ?? '[]')
const sorted = async (page: Page) => (await notes(page)).sort((a, b) => a.start - b.start || a.pitch - b.pitch)
const cursorTick = async (page: Page) => Number(await page.getByTestId('roll-cursor').getAttribute('data-tick'))
const said = (page: Page) => page.getByTestId('roll-said')
const trackPeak = async (page: Page) => Number(((await page.getByTestId('feedback').getAttribute('data-track-peaks')) ?? '').split(',')[0])

test('step entry: a phrase, a chord, a rest, a longer step and Backspace, one undo per note', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')

  const button = page.getByTestId('roll-step-entry')
  await expect(button).toHaveAttribute('aria-pressed', 'false')
  await button.click()
  await expect(button).toHaveAttribute('aria-pressed', 'true')
  await expect(said(page)).toContainText(/Step entry on|Saisie pas à pas activée/)
  await expect(page.getByTestId('roll-cursor')).toBeFocused()

  // A phrase of four notes: A S D F are C4 D4 E4 F4 at the start of the clip, each at the cursor, which moves on.
  expect(await cursorTick(page)).toBe(0)
  await page.keyboard.down('KeyA')
  // The note sounds when written.
  await expect.poll(() => trackPeak(page), { message: 'the note sounds as it is written' }).toBeGreaterThan(0.05)
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  await expect(said(page)).toHaveText(/^C4, bar 1 beat 1; cursor bar 1 beat 1\.25/)
  // The cursor has not moved while the letter is held.
  expect(await cursorTick(page)).toBe(0)
  await page.keyboard.up('KeyA')
  for (const code of ['KeyS', 'KeyD', 'KeyF']) await page.keyboard.press(code)
  await expect(roll(page)).toHaveAttribute('data-note-count', '4')
  const phrase = await sorted(page)
  const unit = phrase[0]!.length
  expect(phrase.map((n) => n.pitch)).toEqual([60, 62, 64, 65])
  expect(phrase.map((n) => n.start)).toEqual([0, unit, 2 * unit, 3 * unit])
  expect(await cursorTick(page)).toBe(4 * unit)

  // A chord: two letters held together are one step; the cursor advances on release of the last.
  await page.keyboard.down('KeyG')
  await page.keyboard.down('KeyH')
  await expect(roll(page)).toHaveAttribute('data-note-count', '6')
  expect(await cursorTick(page)).toBe(4 * unit)
  await page.keyboard.up('KeyG')
  expect(await cursorTick(page)).toBe(4 * unit)
  await page.keyboard.up('KeyH')
  await expect.poll(() => cursorTick(page)).toBe(5 * unit)
  const chord = (await sorted(page)).filter((n) => n.start === 4 * unit)
  expect(chord.map((n) => n.pitch)).toEqual([67, 69])

  // A rest: the cursor moves on, nothing is written. The next note lands after the silence.
  await page.keyboard.press('Period')
  await expect(said(page)).toContainText(/Rest|Silence/)
  await expect.poll(() => cursorTick(page)).toBe(6 * unit)
  await expect(roll(page)).toHaveAttribute('data-note-count', '6')
  await page.keyboard.press('KeyK')
  await expect(roll(page)).toHaveAttribute('data-note-count', '7')
  await expect.poll(() => cursorTick(page)).toBe(7 * unit)
  const after = (await sorted(page)).find((n) => n.start === 6 * unit)!
  expect(after.pitch).toBe(72)

  // Lengthen: the last note grows by the toolbar's note length and the cursor goes to its end.
  await page.keyboard.press('Comma')
  await expect.poll(async () => (await sorted(page)).find((n) => n.start === 6 * unit)!.length).toBeGreaterThan(unit)
  const longer = (await sorted(page)).find((n) => n.start === 6 * unit)!
  await expect.poll(() => cursorTick(page)).toBe(6 * unit + longer.length)

  // Backspace removes that step (not the selection, not a note before it) and puts the cursor back.
  await page.keyboard.press('Backspace')
  await expect(roll(page)).toHaveAttribute('data-note-count', '6')
  await expect.poll(() => cursorTick(page)).toBe(6 * unit)
  await expect(said(page)).toContainText(/Step removed|Pas supprimé/)

  // One undo per command: Backspace, then the lengthening, then the note K, then one chord note at a time, then the phrase.
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '7')
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await sorted(page)).find((n) => n.start === 6 * unit)!.length).toBe(unit)
  for (const count of [6, 5, 4, 3, 2, 1, 0]) {
    await page.keyboard.press('Control+z')
    await expect(roll(page)).toHaveAttribute('data-note-count', String(count))
  }

  // Off with Alt+E: the letter plays and writes nothing; on again, it writes.
  await page.keyboard.press('Alt+KeyE')
  await expect(button).toHaveAttribute('aria-pressed', 'false')
  await page.keyboard.press('KeyA')
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await page.keyboard.press('Alt+KeyE')
  await expect(button).toHaveAttribute('aria-pressed', 'true')

  expect(seen.errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

test('step entry: Backspace after an undo removes the note the clip holds now', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await page.getByTestId('roll-step-entry').click()
  await page.keyboard.press('KeyA')
  await page.keyboard.press('Comma')
  await expect.poll(async () => (await notes(page))[0]!.length).toBeGreaterThan(240)
  // The lengthening is undone: the written note is the clip's shorter one, and Backspace takes that one away.
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await notes(page))[0]!.length).toBe(240)
  await page.keyboard.press('Backspace')
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await expect(said(page)).toContainText(/Step removed|Pas supprimé/)
  expect(seen.errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

test('step entry: rests reach the end of the clip and say so, and a cursor moved by hand makes Backspace the usual one', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await page.getByTestId('roll-step-entry').click()
  await page.keyboard.press('KeyA')
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  // The cursor moved by hand: the step written is forgotten, Backspace removes the (empty) selection and no note.
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('Backspace')
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  // Rests to the end of the one-bar clip: the last one says there is no room.
  for (let i = 0; i < 40; i++) await page.keyboard.press('Period')
  await expect(said(page)).toContainText(/End of the clip|Fin du clip/)
  await page.keyboard.press('KeyA')
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  expect(seen.errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

// #899: the mode is global like musical typing, shown beside the note counter, and Comma lengthens by the toolbar's length.
test('step entry: stays on from clip to clip and says when it stops, and the indicator shows whether letters write or play', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await page.getByTestId('nav-arrangement').click()
  await page.getByTestId('add-track').click()
  await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Drums', exact: true }).click()
  await expect(page.getByTestId('arr-track')).toHaveCount(2)
  await page.getByTestId('arr-track').first().click()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  const button = page.getByTestId('roll-step-entry')
  const letters = page.getByTestId('roll-letters')
  await expect(letters).toHaveAttribute('data-mode', 'play')
  await expect(letters).toHaveText(/Letters play|Les lettres jouent/)
  await button.click()
  await expect(letters).toHaveAttribute('data-mode', 'write')
  await expect(letters).toHaveText(/Letters write|Les lettres écrivent/)

  // A track with no clip: the roll has none to write into, the mode stops, and the page says so.
  await page.getByTestId('arr-track').nth(1).click()
  await expect(page.getByTestId('roll-stop-said')).toContainText(/Step entry off|Saisie pas à pas désactivée/)
  // Back on the first track the mode is off and the indicator says the letters play.
  await page.getByTestId('arr-track').first().click()
  await expect(button).toHaveAttribute('aria-pressed', 'false')
  await expect(letters).toHaveAttribute('data-mode', 'play')

  // An audio track has no notes either: the mode stops there too, and it is said there. The indicator does not move the toolbar.
  const before = await page.getByTestId('roll-count').boundingBox()
  await button.click()
  const during = await page.getByTestId('roll-count').boundingBox()
  expect(during!.y).toBe(before!.y)
  await page.getByTestId('add-track').click()
  await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Audio track' }).click()
  await expect(page.getByTestId('roll-stop-said')).toContainText(/Step entry off|Saisie pas à pas désactivée/)
  await page.getByTestId('arr-track').first().click()
  await expect(button).toHaveAttribute('aria-pressed', 'false')

  // A clip on the second track, the mode on: going back to the first track's clip keeps it on, and the letters write there.
  await page.getByTestId('arr-track').nth(1).click()
  await page.getByTestId('roll-create-clip').click()
  await button.click()
  await expect(button).toHaveAttribute('aria-pressed', 'true')
  await page.getByTestId('arr-track').first().click()
  await expect(button).toHaveAttribute('aria-pressed', 'true')
  await expect(letters).toHaveAttribute('data-mode', 'write')
  await page.keyboard.press('KeyA')
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')

  // Musical typing off ends the mode, said, and the indicator follows.
  await page.keyboard.press('Backquote')
  await expect(button).toHaveAttribute('aria-pressed', 'false')
  await expect(said(page)).toContainText(/Step entry off|Saisie pas à pas désactivée/)
  await expect(letters).toHaveAttribute('data-mode', 'off')
  expect(seen.errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

test('step entry: Comma lengthens by the toolbar length, whatever the snap, and the cursor stays on that grid', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await page.getByTestId('roll-draw-length').selectOption('eighth')
  await expect(page.getByTestId('roll-snap')).toHaveValue('auto')
  await page.getByTestId('roll-step-entry').click()
  await page.keyboard.press('KeyS')
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  const eighth = (await notes(page))[0]!.length
  await page.keyboard.press('Comma')
  await page.keyboard.press('Comma')
  await page.keyboard.press('Comma')
  await expect.poll(async () => (await notes(page))[0]!.length).toBe(4 * eighth)
  expect(await cursorTick(page)).toBe(4 * eighth)
  await expect(said(page)).toContainText(/cursor bar 1 beat 3|curseur mesure 1 temps 3/)
  expect(seen.errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})
