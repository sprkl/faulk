import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// PN-2 (#83): advanced editing of the piano roll in a real browser: quantize with a strength,
// transpose, duplicate, copy, cut and paste, by the toolbar and by the keyboard. Each action is one
// command and one history entry. The notes are read back from the roll's data attributes.

const ROW = 16
const RULER = 28

interface Note {
  start: number
  pitch: number
  length: number
  velocity: number
}

const roll = (page: Page) => page.getByTestId('piano-roll')
const stage = (page: Page) => page.getByTestId('roll-stage')
const notes = async (page: Page): Promise<Note[]> => JSON.parse((await roll(page).getAttribute('data-notes')) ?? '[]')
const starts = async (page: Page) => (await notes(page)).map((n) => n.start)
const pitches = async (page: Page) => (await notes(page)).map((n) => n.pitch)
const num = async (page: Page, testid: string, attribute: string) => Number(await page.getByTestId(testid).getAttribute(attribute))

async function spot(page: Page) {
  const box = (await stage(page).boundingBox())!
  const ppt = await num(page, 'timegrid', 'data-px-per-tick')
  const scrollY = await num(page, 'roll-stage', 'data-scroll-y')
  return { x: (tick: number) => box.x + tick * ppt, y: (pitch: number) => box.y + RULER + (127 - pitch) * ROW - scrollY + ROW / 2 }
}

async function openRoll(page: Page, lang?: 'fr') {
  const seen = watch(page)
  const warnings: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'warning') warnings.push(message.text())
  })
  if (lang) await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width: 1400, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  return { seen, warnings }
}

/** The start `clip.notes.quantize` gives, written again here from the definition (nearest line, halfway later, share rounded away from zero). */
function expected(start: number, grid: number, strength: number, length: number, clip: number) {
  const nearest = Math.floor((start + grid / 2) / grid) * grid
  const scaled = (nearest - start) * strength
  const share = Math.floor((Math.abs(scaled) + 50) / 100)
  return Math.min(Math.max(0, start + (scaled < 0 ? -share : share)), clip - length)
}

const focusGrid = (page: Page) => page.getByTestId('timegrid').focus()

test('piano roll: quantizes with a strength, transposes, duplicates, copies, cuts and pastes, each as one history entry', async ({ page }) => {
  test.setTimeout(120_000)
  const { seen, warnings } = await openRoll(page)
  await page.getByTestId('roll-snap').selectOption('off')

  // Three notes off the grid (snap off: a click draws at the tick under the pointer).
  const g = await spot(page)
  await page.mouse.click(g.x(130), g.y(60))
  await page.mouse.click(g.x(1000), g.y(62))
  await page.mouse.click(g.x(2100), g.y(64))
  const drawn = await notes(page)
  expect(drawn).toHaveLength(3)
  const original = drawn.map((n) => n.start)
  expect(original.every((s) => s % 960 !== 0)).toBe(true)

  // Quantize with no selection takes every note (the last drawn note is selected: Escape clears it); half strength moves each half of the way.
  await focusGrid(page)
  await page.keyboard.press('Escape')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')
  await page.getByTestId('roll-quantize-grid').selectOption('quarter')
  await page.getByTestId('roll-quantize-strength').fill('50')
  await page.getByTestId('roll-quantize-strength').blur()
  await page.getByTestId('roll-quantize').click()
  const half = original.map((s, i) => expected(s, 960, 50, drawn[i]!.length, 3840))
  await expect.poll(() => starts(page)).toEqual(half)
  // One undo takes the whole quantize back.
  await page.keyboard.press('Control+z')
  await expect.poll(() => starts(page)).toEqual(original)

  // Strength 0 changes nothing and records nothing: the undo below still reaches the previous entry.
  await page.getByTestId('roll-quantize-strength').fill('0')
  await page.getByTestId('roll-quantize-strength').blur()
  await page.getByTestId('roll-quantize').click()
  expect(await starts(page)).toEqual(original)
  await page.getByTestId('roll-quantize-strength').fill('100')
  await page.getByTestId('roll-quantize-strength').blur()
  await page.getByTestId('roll-quantize').click()
  const full = original.map((s, i) => expected(s, 960, 100, drawn[i]!.length, 3840))
  await expect.poll(() => starts(page)).toEqual(full)
  expect(full.every((s) => s % 960 === 0)).toBe(true)

  // Transpose by the keyboard: the grid is focused, all notes selected. Up is a semitone, Shift an octave.
  await focusGrid(page)
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '3')
  await page.keyboard.press('ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([61, 63, 65])
  await page.keyboard.press('Shift+ArrowDown')
  await expect.poll(() => pitches(page)).toEqual([49, 51, 53])
  await page.keyboard.press('Control+z')
  await expect.poll(() => pitches(page)).toEqual([61, 63, 65])
  await page.keyboard.press('Control+z')
  await expect.poll(() => pitches(page)).toEqual([60, 62, 64])
  // The buttons do the same (an undo leaves the selection on notes that are gone: select again).
  await focusGrid(page)
  await page.keyboard.press('Control+a')
  await page.getByTestId('roll-transpose-up').click()
  await expect.poll(() => pitches(page)).toEqual([61, 63, 65])
  await page.getByTestId('roll-transpose-down').click()
  await expect.poll(() => pitches(page)).toEqual([60, 62, 64])
  // Past the top of the keyboard nothing is sent: the notes stay.
  await focusGrid(page)
  for (let i = 0; i < 4; i++) await page.keyboard.press('Shift+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([108, 110, 112])
  await page.keyboard.press('Shift+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([120, 122, 124])
  await page.keyboard.press('Shift+ArrowUp')
  expect(await pitches(page)).toEqual([120, 122, 124])
  for (let i = 0; i < 5; i++) await page.keyboard.press('Control+z')
  await expect.poll(() => pitches(page)).toEqual([60, 62, 64])

  // Duplicate the selection: the copies follow it, and the clip grows by whole bars when they do not fit, in one entry.
  const before = await notes(page)
  await focusGrid(page)
  await page.keyboard.press('Control+a')
  await page.keyboard.press('Control+d')
  await expect(roll(page)).toHaveAttribute('data-clip-length', '7680')
  await expect(roll(page)).toHaveAttribute('data-note-count', '6')
  // The copies go a whole bar later (the notes are all in the first bar).
  const span = 3840
  expect(await starts(page)).toEqual([...before.map((n) => n.start), ...before.map((n) => n.start + span)].sort((a, b) => a - b))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '3')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-clip-length', '3840')
  await expect(roll(page)).toHaveAttribute('data-note-count', '3')
  // A single note is copied a whole bar later too, so the one-bar clip grows; the button does the same as the key.
  await page.getByTestId('roll-tool-select').click()
  await page.mouse.click(g.x(full[0]! + 20), g.y(60))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  await page.getByTestId('roll-duplicate').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '4')
  await expect(roll(page)).toHaveAttribute('data-clip-length', '7680')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '3')
  await expect(roll(page)).toHaveAttribute('data-clip-length', '3840')

  // Copy, then paste: the copy lands at the start of the clip (no play position), the selection is the pasted note.
  await page.mouse.click(g.x(full[2]! + 20), g.y(64))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  await focusGrid(page)
  await page.keyboard.press('Control+c')
  await page.keyboard.press('ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([60, 62, 65])
  await page.keyboard.press('Control+v')
  await expect(roll(page)).toHaveAttribute('data-note-count', '4')
  expect((await notes(page)).some((n) => n.start === 0 && n.pitch === 64)).toBe(true)
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '3')
  // Cut removes the note and keeps it for a paste.
  await page.mouse.click(g.x(full[1]! + 20), g.y(62))
  await focusGrid(page)
  await page.keyboard.press('Control+x')
  await expect(roll(page)).toHaveAttribute('data-note-count', '2')
  await page.keyboard.press('Control+v')
  await expect(roll(page)).toHaveAttribute('data-note-count', '3')
  expect((await notes(page)).some((n) => n.start === 0 && n.pitch === 62)).toBe(true)

  // The keys of the roll never reach the transport: Space still belongs to it, and no note was played by these shortcuts.
  expect(seen.errors).toEqual([])
  expect(warnings).toEqual([])
})

test('piano roll: the editing controls speak French', async ({ page }) => {
  const { seen } = await openRoll(page, 'fr')
  await expect(page.getByTestId('roll-quantize')).toHaveText('Quantifier')
  await expect(page.getByTestId('roll-duplicate')).toHaveText('Dupliquer')
  await expect(page.getByTestId('roll-quantize-grid')).toHaveAccessibleName('Grille de quantification')
  await expect(page.getByTestId('roll-quantize-strength')).toHaveAccessibleName('Force de la quantification en pourcent')
  await expect(page.getByTestId('roll-transpose-up')).toHaveAccessibleName('Transposer d’un demi-ton vers le haut')
  await expect(page.getByTestId('roll-quantize-grid').locator('option').last()).toHaveText('1/16 triolet')
  await expect(page.getByTestId('roll-quantize')).toBeDisabled()
  expect(seen.errors).toEqual([])
})

test('piano roll: the editing controls speak English', async ({ page }) => {
  const { seen } = await openRoll(page)
  await expect(page.getByTestId('roll-quantize')).toHaveText('Quantize')
  await expect(page.getByTestId('roll-duplicate')).toHaveText('Duplicate')
  await expect(page.getByTestId('roll-quantize-grid').locator('option').last()).toHaveText('1/16 triplet')
  await expect(page.getByTestId('roll-duplicate')).toBeDisabled()
  expect(seen.errors).toEqual([])
})

// #397: the mouse gestures of the roll. At the default zoom the ruler's Auto step is a 32nd; drawing stops at a sixteenth.
test('piano roll: Auto draws on a sixteenth at least, duplicate goes whole bars later, Ctrl+A selects the notes without the page text', async ({ page }) => {
  test.setTimeout(120_000)
  const { seen, warnings } = await openRoll(page)
  await expect(page.getByTestId('roll-snap')).toHaveValue('auto')
  const sixteenth = 240
  const g = await spot(page)

  // A click puts a note on the step, one step long, not a sliver of the finest grid.
  await page.mouse.click(g.x(100), g.y(60))
  expect(await notes(page)).toEqual([{ start: 0, pitch: 60, length: sixteenth, velocity: 100 }])
  // A drag is rounded to the step.
  await page.mouse.move(g.x(1000), g.y(62))
  await page.mouse.down()
  await page.mouse.move(g.x(1300), g.y(62), { steps: 4 })
  await page.mouse.move(g.x(1700), g.y(62), { steps: 4 })
  await page.mouse.up()
  const dragged = (await notes(page)).find((n) => n.pitch === 62)!
  expect(dragged.start % sixteenth).toBe(0)
  expect(dragged.length % sixteenth).toBe(0)
  expect(dragged.length).toBeGreaterThanOrEqual(sixteenth)
  const total = (await notes(page)).length

  // Duplicate: a selection inside the first bar is copied to the second bar, whatever its span.
  await page.getByTestId('roll-tool-select').click()
  await page.mouse.click(g.x(100), g.y(60))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  await page.getByTestId('roll-duplicate').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', String(total + 1))
  expect(await starts(page)).toContain(3840)
  await expect(roll(page)).toHaveAttribute('data-clip-length', '7680')

  // Ctrl+A with the focus on the toolbar (the "Select" button just clicked), not on the grid: every note is selected
  // and no page text is. Ctrl+D then duplicates the selection from there too.
  await focusGrid(page)
  await page.keyboard.press('Escape')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')
  await page.getByTestId('roll-tool-select').click()
  await expect(page.getByTestId('roll-tool-select')).toBeFocused()
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', String(total + 1))
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? '')).toBe('')
  await page.keyboard.press('Control+d')
  await expect(roll(page)).toHaveAttribute('data-note-count', String(2 * (total + 1)))
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', String(total + 1))
  // The same with nothing focused (the page body).
  await focusGrid(page)
  await page.keyboard.press('Escape')
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', String(total + 1))
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? '')).toBe('')

  // In a text field Ctrl+A stays the field's own: the notes are not selected.
  await focusGrid(page)
  await page.keyboard.press('Escape')
  await page.getByTestId('roll-quantize-strength').focus()
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')
  expect(seen.errors).toEqual([])
  expect(warnings).toEqual([])
})

test('piano roll: Q quantizes the selection or the whole clip, says how many notes moved, one history entry each, and the button and the menu show the key', async ({
  page,
}) => {
  test.setTimeout(120_000)
  const { seen, warnings } = await openRoll(page)
  await page.getByTestId('roll-snap').selectOption('off')
  await page.getByTestId('roll-quantize-grid').selectOption('quarter')
  const g = await spot(page)
  await page.mouse.click(g.x(130), g.y(60))
  await page.mouse.click(g.x(1100), g.y(62))
  await page.mouse.click(g.x(2100), g.y(64))
  const drawn = (await starts(page)).slice().sort((a, b) => a - b)
  expect(drawn).toHaveLength(3)
  const done = () => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[] } }).__faulkHistory().undo.length)

  // The key is on the button's tooltip.
  await expect(page.getByTestId('roll-quantize')).toHaveAttribute('title', /\(Q\)/)

  // Partial selection: only the chosen note moves, one entry, and the count is shown.
  await focusGrid(page)
  await page.keyboard.press('Escape')
  await page.mouse.click(g.x(1100 + 10), g.y(62))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  const before = await done()
  await focusGrid(page)
  await page.keyboard.press('KeyQ')
  await expect.poll(async () => (await starts(page)).slice().sort((a, b) => a - b)).not.toEqual(drawn)
  expect(await done()).toBe(before + 1)
  await expect(page.getByTestId('roll-quantize-said')).toContainText(/1/)
  const after = (await starts(page)).slice().sort((a, b) => a - b)
  expect(after.filter((s) => s % 960 === 0)).toHaveLength(1)
  expect(after).toContain(130)
  expect(after).toContain(2100)

  // Again: the note is on the grid already, nothing is sent, and it says so.
  await page.keyboard.press('KeyQ')
  await expect(page.getByTestId('roll-quantize-said')).toContainText(/Already on the grid|Déjà sur la grille/)
  expect(await done()).toBe(before + 1)

  // Undo takes it back whole.
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await starts(page)).slice().sort((a, b) => a - b)).toEqual(drawn)

  // No selection: every note.
  await page.keyboard.press('Escape')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')
  await page.keyboard.press('KeyQ')
  await expect.poll(async () => (await starts(page)).every((s) => s % 960 === 0)).toBe(true)
  await expect(page.getByTestId('roll-quantize-said')).toContainText(/3/)

  // The menu: the entry shows the key.
  await page.mouse.click(g.x(2500), g.y(64), { button: 'right' })
  await expect(page.getByTestId('ctx-quantize')).toContainText('Q')
  await page.keyboard.press('Escape')

  // Q with the focus in a field types, it does not quantize.
  await page.getByTestId('roll-quantize-strength').focus()
  const n = await done()
  await page.keyboard.press('KeyQ')
  expect(await done()).toBe(n)

  expect(seen.errors).toEqual([])
  expect(warnings).toEqual([])
})
