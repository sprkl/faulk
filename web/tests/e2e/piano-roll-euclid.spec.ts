import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #694: the euclidean rhythm generator of the piano roll, in a real browser and at the keyboard. It is a page tool: what it
// writes is `clip.notes.add` (and the removal of the same-pitch notes it replaces, in one history group), so one undo takes
// it all back. The notes are read back from the roll's data attributes.

const SIXTEENTH = 240

interface Note {
  start: number
  pitch: number
  length: number
  velocity: number
}

const roll = (page: Page) => page.getByTestId('piano-roll')
const notes = async (page: Page): Promise<Note[]> => JSON.parse((await roll(page).getAttribute('data-notes')) ?? '[]')
const shape = async (page: Page) => (await notes(page)).map((n) => [n.start / SIXTEENTH, n.pitch, n.length])
const panel = (page: Page) => page.getByTestId('roll-euclid')
const summary = (page: Page) => page.getByTestId('roll-euclid-summary')

async function openRoll(page: Page, options: { lang?: 'fr'; drums?: boolean } = {}) {
  const seen = watch(page)
  if (options.lang) await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), options.lang)
  await page.setViewportSize({ width: 1400, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  if (options.drums) {
    await page.getByTestId('add-track').click()
    await page.getByRole('menuitem', { name: 'Drums', exact: true }).click()
  }
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  return seen
}

/** The generator opened from the toolbar's "Generate" list, with the mouse (the keyboard way is tested apart). */
async function openFromToolbar(page: Page, name: string) {
  await page.getByTestId('roll-generate').click()
  await page.getByRole('menuitem', { name }).click()
  await expect(panel(page)).toBeVisible()
  await expect(page.getByTestId('roll-euclid-steps')).toBeFocused()
}

test('piano roll: the euclidean generator places E(5, 8) at the keyboard, previews it, replaces what it covers, and one undo takes it all back', async ({
  page,
}) => {
  test.setTimeout(120_000)
  const seen = await openRoll(page)
  const said = page.getByTestId('roll-said')
  await expect(page.getByTestId('roll-generate')).toHaveText('Generate')

  // The toolbar's list holds the generator, with its key.
  await page.getByTestId('roll-generate').click()
  await expect(page.getByRole('menuitem', { name: 'Euclidean…' })).toContainText('Alt+G')
  await page.keyboard.press('Escape')
  await expect(page.getByRole('menuitem', { name: 'Euclidean…' })).toHaveCount(0)

  // Alt+G from the grid opens it, on the first field. Steps 8, hits 5, then Enter: five notes of E(5, 8) = x.xx.xx.
  await page.getByTestId('timegrid').focus()
  await page.keyboard.press('Alt+g')
  await expect(panel(page)).toBeVisible()
  await expect(page.getByTestId('roll-euclid-steps')).toBeFocused()
  await expect(page.getByTestId('roll-euclid-grid')).toHaveValue('sixteenth')
  await page.keyboard.type('8')
  await page.keyboard.press('Tab')
  await page.keyboard.type('5')
  // The numbers drive the summary while they change; nothing is written yet.
  await expect(summary(page)).toContainText('5 notes on')
  expect(await notes(page)).toEqual([])
  await page.keyboard.press('Enter')
  await expect(panel(page)).toHaveCount(0)
  await expect(roll(page)).toHaveAttribute('data-note-count', '5')
  const pitch = (await notes(page))[0]!.pitch
  expect(await shape(page)).toEqual([0, 2, 3, 5, 6].map((s) => [s, pitch, SIXTEENTH / 2]))
  expect((await notes(page)).every((n) => n.velocity === 100)).toBe(true)
  await expect(said).toContainText('Placed: 5 new notes on')
  // The new notes are the selection, and the focus is back on what opened the generator.
  await expect(roll(page)).toHaveAttribute('data-selected-count', '5')
  await expect(page.getByTestId('timegrid')).toBeFocused()

  // One undo removes all five.
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await page.keyboard.press('Control+Shift+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '5')

  // E(3, 8) on the same pitch replaces the five (the three notes that are already right stay, the two others go), and says so before it does.
  await openFromToolbar(page, 'Euclidean…')
  await expect(page.getByTestId('roll-euclid-steps')).toHaveValue('8')
  await expect(page.getByTestId('roll-euclid-hits')).toHaveValue('5')
  await page.getByTestId('roll-euclid-hits').fill('3')
  await expect(summary(page)).toContainText('replacing 2')
  await page.getByTestId('roll-euclid-submit').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '3')
  expect(await shape(page)).toEqual([0, 3, 6].map((s) => [s, pitch, SIXTEENTH / 2]))
  await expect(said).toContainText('Placed: 0 new notes')
  await expect(said).toContainText('2 replaced')
  // One undo brings the five back, the three gone.
  await page.keyboard.press('Control+z')
  expect(await shape(page)).toEqual([0, 2, 3, 5, 6].map((s) => [s, pitch, SIXTEENTH / 2]))

  // "Add without replacing": the five stay and E(3, 8) turned by one step is added next to them.
  await page.getByTestId('timegrid').focus()
  await page.keyboard.press('Alt+g')
  await page.getByTestId('roll-euclid-hits').fill('3')
  await page.getByTestId('roll-euclid-rotation').fill('1')
  await page.getByTestId('roll-euclid-keep').check()
  await expect(summary(page)).not.toContainText('replacing')
  await page.keyboard.press('Enter')
  await expect(roll(page)).toHaveAttribute('data-note-count', '8')
  expect((await shape(page)).map((n) => n[0]).sort((a, b) => a! - b!)).toEqual([0, 1, 2, 3, 4, 5, 6, 7])
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '5')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')

  // Escape closes with nothing placed; a field out of range names itself, disables Place and Enter writes nothing.
  await page.getByTestId('timegrid').focus()
  await page.keyboard.press('Alt+g')
  await page.keyboard.type('1')
  await expect(summary(page)).toHaveText('Steps: a whole number from 2 to 64.')
  await expect(page.getByTestId('roll-euclid-submit')).toBeDisabled()
  await page.keyboard.press('Enter')
  await expect(panel(page)).toBeVisible()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await page.keyboard.press('Backspace')
  await page.keyboard.type('4')
  await page.keyboard.press('Tab')
  await page.keyboard.type('9')
  await expect(summary(page)).toHaveText('Hits: a whole number from 1 to the number of steps.')
  await page.keyboard.press('Escape')
  await expect(panel(page)).toHaveCount(0)
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await expect(page.getByTestId('timegrid')).toBeFocused()

  // A letter typed on a tick box plays nothing and opens nothing (the keys of the strip stay in the strip).
  await page.keyboard.press('Alt+g')
  await page.getByTestId('roll-euclid-fill').focus()
  await page.keyboard.press('q')
  await expect(panel(page)).toBeVisible()
  await page.keyboard.press('Escape')

  expect(seen.errors).toEqual([])
})

test('piano roll: "Fill the clip" repeats E(3, 8) over a clip of two bars, one history entry, and the length is a share of the step', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = await openRoll(page)
  await page.getByTestId('roll-longer').click()
  await expect(page.getByTestId('roll-length')).toHaveValue('2')
  await page.getByTestId('timegrid').focus()
  await page.keyboard.press('Alt+g')
  // 2 bars in sixteenths: 32 steps, E(3, 8) four times.
  await page.getByTestId('roll-euclid-fill').check()
  await page.getByTestId('roll-euclid-gate').fill('25')
  await page.getByTestId('roll-euclid-velocity').fill('90')
  await expect(summary(page)).toContainText('12 notes on')
  await page.keyboard.press('Enter')
  await expect(roll(page)).toHaveAttribute('data-note-count', '12')
  const placed = await notes(page)
  expect(placed.map((n) => n.start / SIXTEENTH)).toEqual([0, 3, 6, 8, 11, 14, 16, 19, 22, 24, 27, 30])
  expect(placed.every((n) => n.length === SIXTEENTH / 4 && n.velocity === 90)).toBe(true)
  // One undo removes the twelve.
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  expect(seen.errors).toEqual([])
})

test('piano roll: on a Drums track the generator writes the chosen sound, from a right click, and a note under the pattern at the other pitch stays', async ({
  page,
}) => {
  test.setTimeout(120_000)
  const seen = await openRoll(page, { drums: true })
  await page.getByTestId('roll-stage').click({ button: 'right', position: { x: 200, y: 200 } })
  await page.getByRole('menuitem', { name: 'Euclidean…' }).click()
  await expect(panel(page)).toBeVisible()
  // The pitch field is a list of the kit's sounds.
  const sound = page.getByTestId('roll-euclid-pitch')
  await expect(sound).toHaveJSProperty('tagName', 'SELECT')
  await expect(sound.locator('option')).toHaveText(['Kick', 'Snare', 'Clap', 'Closed Hat', 'Open Hat'])
  await sound.selectOption({ label: 'Snare' })
  await page.getByTestId('roll-euclid-hits').fill('4')
  await page.getByTestId('roll-euclid-steps').fill('16')
  await expect(summary(page)).toContainText('4 notes on Snare')
  await page.keyboard.press('Enter')
  await expect(roll(page)).toHaveAttribute('data-note-count', '4')
  expect(await shape(page)).toEqual([0, 4, 8, 12].map((s) => [s, 38, SIXTEENTH / 2]))

  // A kick pattern over the same steps replaces nothing: the snares are at another pitch.
  await page.getByTestId('timegrid').focus()
  await page.keyboard.press('Alt+g')
  await page.getByTestId('roll-euclid-pitch').selectOption({ label: 'Kick' })
  await page.getByTestId('roll-euclid-submit').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '8')
  expect((await notes(page)).filter((n) => n.pitch === 38)).toHaveLength(4)
  expect((await notes(page)).filter((n) => n.pitch === 36)).toHaveLength(4)
  expect(seen.errors).toEqual([])
})

test('piano roll: the euclidean generator speaks French', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = await openRoll(page, { lang: 'fr' })
  await expect(page.getByTestId('roll-generate')).toHaveText('Générer')
  await openFromToolbar(page, 'Euclidien…')
  await expect(panel(page)).toHaveAttribute('aria-label', 'Rythme euclidien')
  await expect(panel(page)).toContainText('Pas')
  await expect(panel(page)).toContainText('Coups')
  await expect(panel(page)).toContainText('Remplir le clip')
  await expect(panel(page)).toContainText('Ajouter sans remplacer')
  await expect(page.getByTestId('roll-euclid-submit')).toHaveText('Poser')
  await page.keyboard.type('8')
  await page.keyboard.press('Tab')
  await page.keyboard.type('5')
  await expect(summary(page)).toContainText('5 notes sur')
  await page.keyboard.press('Tab')
  await page.keyboard.press('Tab')
  await page.keyboard.press('Tab')
  await page.keyboard.type('x')
  await expect(summary(page)).toHaveText('Hauteur : une note comme C2 ou F#3, ou un nombre de 0 à 127.')
  await page.keyboard.press('Escape')
  await page.keyboard.press('Alt+g')
  await page.keyboard.type('8')
  await page.keyboard.press('Tab')
  await page.keyboard.type('5')
  await page.keyboard.press('Enter')
  await expect(roll(page)).toHaveAttribute('data-note-count', '5')
  await expect(page.getByTestId('roll-said')).toContainText('Posé : 5 nouvelles notes sur')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  expect(seen.errors).toEqual([])
})
