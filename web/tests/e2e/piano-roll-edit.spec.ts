import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

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
  await engineUp(page)
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
  const stageTop = (await stage(page).boundingBox())!.y
  await page.getByTestId('roll-quantize').click()
  const half = original.map((s, i) => expected(s, 960, 50, drawn[i]!.length, 3840))
  await expect.poll(() => starts(page)).toEqual(half)
  // What the quantize says floats under its field: the toolbar keeps its lines and the grid does not move (#623).
  await expect(page.getByTestId('roll-quantize-said')).toBeVisible()
  expect((await stage(page).boundingBox())!.y).toBe(stageTop)
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

  // Transpose by the keyboard: the grid is focused, all notes selected. Alt+Up is a semitone, Alt+Shift an octave.
  await focusGrid(page)
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '3')
  await page.keyboard.press('Alt+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([61, 63, 65])
  await page.keyboard.press('Alt+Shift+ArrowDown')
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
  for (let i = 0; i < 4; i++) await page.keyboard.press('Alt+Shift+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([108, 110, 112])
  await page.keyboard.press('Alt+Shift+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([120, 122, 124])
  await page.keyboard.press('Alt+Shift+ArrowUp')
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
  await page.keyboard.press('Alt+ArrowUp')
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
  await expect(page.getByTestId('roll-draw-length')).toHaveAccessibleName('Longueur des notes dessinées')
  await expect(page.getByTestId('roll-draw-length').locator('option:checked')).toHaveText('Magnétisme')
  await expect(page.getByTestId('roll-draw-length').locator('option').last()).toHaveText('4 mesures')
  // #711: the snap's triplets, named as the quantize grid names them, and the Length field's.
  await expect(page.getByTestId('roll-snap').locator('option').last()).toHaveText('1/16 triolet')
  await expect(page.getByTestId('roll-draw-length').locator('option[value="eighth-triplet"]')).toHaveText('Triolet de croches')
  await expect(page.getByTestId('roll-quantize')).toBeDisabled()
  expect(seen.errors).toEqual([])
})

test('piano roll: the editing controls speak English', async ({ page }) => {
  const { seen } = await openRoll(page)
  await expect(page.getByTestId('roll-quantize')).toHaveText('Quantize')
  await expect(page.getByTestId('roll-duplicate')).toHaveText('Duplicate')
  await expect(page.getByTestId('roll-quantize-grid').locator('option').last()).toHaveText('1/16 triplet')
  await expect(page.getByTestId('roll-draw-length')).toHaveAccessibleName('Length of drawn notes')
  await expect(page.getByTestId('roll-draw-length').locator('option:checked')).toHaveText('Snap')
  await expect(page.getByTestId('roll-draw-length').locator('option').last()).toHaveText('4 bars')
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

  // The key is on the button's tooltip.
  await expect(page.getByTestId('roll-quantize')).toHaveAttribute('title', /\(Q\)/)

  // Partial selection: only the chosen note moves, one entry, and the count is shown.
  await focusGrid(page)
  await page.keyboard.press('Escape')
  await page.mouse.click(g.x(1100 + 10), g.y(62))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  await focusGrid(page)
  await page.keyboard.press('KeyQ')
  await expect.poll(async () => (await starts(page)).slice().sort((a, b) => a - b)).not.toEqual(drawn)
  await expect(page.getByTestId('roll-quantize-said')).toContainText(/1/)
  const after = (await starts(page)).slice().sort((a, b) => a - b)
  expect(after.filter((s) => s % 960 === 0)).toHaveLength(1)
  expect(after).toContain(130)
  expect(after).toContain(2100)

  // Again: the note is on the grid already, nothing is sent, and it says so.
  await page.keyboard.press('KeyQ')
  await expect(page.getByTestId('roll-quantize-said')).toContainText(/Already on the grid|Déjà sur la grille/)
  expect((await starts(page)).slice().sort((a, b) => a - b)).toEqual(after)

  // Undo takes it back whole: one entry.
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await starts(page)).slice().sort((a, b) => a - b)).toEqual(drawn)

  // No selection: every note, and one undo takes all three back (one history entry).
  await page.keyboard.press('Escape')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')
  await page.keyboard.press('KeyQ')
  await expect.poll(async () => (await starts(page)).every((s) => s % 960 === 0)).toBe(true)
  await expect(page.getByTestId('roll-quantize-said')).toContainText(/3/)
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await starts(page)).slice().sort((a, b) => a - b)).toEqual(drawn)
  await page.keyboard.press('Control+Shift+z')
  await expect.poll(async () => (await starts(page)).every((s) => s % 960 === 0)).toBe(true)

  // The menu: the entry shows the key.
  await page.mouse.click(g.x(2500), g.y(64), { button: 'right' })
  await expect(page.getByTestId('ctx-quantize')).toContainText('Q')
  await page.keyboard.press('Escape')

  // Q with the focus in a field types, it does not quantize: undo first so that notes are off the grid, then press Q in the field.
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await starts(page)).slice().sort((a, b) => a - b)).toEqual(drawn)
  await page.getByTestId('roll-quantize-strength').focus()
  await page.keyboard.press('KeyQ')
  await page.waitForTimeout(300)
  expect((await starts(page)).slice().sort((a, b) => a - b)).toEqual(drawn)
  await expect(page.getByTestId('roll-quantize-said')).toHaveCount(0)

  expect(seen.errors).toEqual([])
  expect(warnings).toEqual([])
})

test('piano roll: on an AZERTY layout the quantize key is labelled with the character on it, in the tooltip, the menu and the shortcut list, and it still works by position', async ({
  page,
}) => {
  test.setTimeout(120_000)
  await page.addInitScript(() => {
    const layout = new Map([['KeyQ', 'a']])
    Object.defineProperty(navigator, 'keyboard', {
      configurable: true,
      value: { getLayoutMap: () => Promise.resolve(layout), addEventListener: () => {}, removeEventListener: () => {} },
    })
  })
  const { seen } = await openRoll(page)
  await page.getByTestId('roll-snap').selectOption('off')
  await page.getByTestId('roll-quantize-grid').selectOption('quarter')
  const g = await spot(page)
  await page.mouse.click(g.x(130), g.y(60))
  await expect(page.getByTestId('roll-quantize')).toHaveAttribute('title', /\(A\)$/)
  await expect(page.getByTestId('roll-quantize')).toHaveAttribute('aria-keyshortcuts', 'A')
  await page.mouse.click(g.x(150), g.y(60), { button: 'right' })
  await expect(page.getByTestId('ctx-quantize')).toContainText('A')
  await page.keyboard.press('Escape')
  // The key by position still quantizes.
  await focusGrid(page)
  await page.keyboard.press('KeyQ')
  await expect.poll(async () => starts(page)).toEqual([0])
  // The shortcut list writes it the same way.
  await page.keyboard.press('?')
  const line = page.getByRole('dialog').locator('li', { hasText: /Quantize|Quantifier/ })
  await expect(line.locator('kbd')).toHaveText('A')
  expect(seen.errors).toEqual([])
})

// #529: the shortcut list says what the arrows do, and the transposition it lists works on the selection, one entry per gesture.
test('piano roll: Alt+Up and Alt+Down transpose the selection (Alt+Shift: an octave), plain arrows only move between notes, and the list says so', async ({
  page,
}) => {
  test.setTimeout(120_000)
  const { seen } = await openRoll(page)
  const g = await spot(page)
  await page.mouse.click(g.x(100), g.y(60))
  await page.mouse.click(g.x(1000), g.y(64))
  await focusGrid(page)
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '2')

  // Plain Up and Shift+Up do not transpose.
  await page.keyboard.press('ArrowUp')
  await page.keyboard.press('Shift+ArrowUp')
  await page.waitForTimeout(400)
  expect(await pitches(page)).toEqual([60, 64])

  await page.keyboard.press('Alt+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([61, 65])
  await page.keyboard.press('Alt+Shift+ArrowDown')
  await expect.poll(() => pitches(page)).toEqual([49, 53])
  // One history entry per gesture: one undo takes one gesture back.
  await page.keyboard.press('Control+z')
  await expect.poll(() => pitches(page)).toEqual([61, 65])
  await page.keyboard.press('Control+z')
  await expect.poll(() => pitches(page)).toEqual([60, 64])

  // From a note's cell and from the free cursor too.
  await page.keyboard.press('Control+a')
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Alt+ArrowDown')
  await expect.poll(() => pitches(page)).toEqual([59, 63])
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('roll-cursor')).toBeFocused()
  await page.keyboard.press('Alt+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([60, 64])
  // Each of those was one entry: two undos take both back.
  await page.keyboard.press('Control+z')
  await expect.poll(() => pitches(page)).toEqual([59, 63])
  await page.keyboard.press('Control+z')
  await expect.poll(() => pitches(page)).toEqual([60, 64])

  // The list and the gesture agree.
  await page.keyboard.press('?')
  const dialog = page.getByRole('dialog')
  const transpose = dialog.locator('li', { hasText: /Transpose the selected notes/ })
  await expect(transpose.locator('kbd')).toHaveText(['Alt', '↑', 'Alt', '↓', 'Alt', 'Shift', '↑', 'Alt', 'Shift', '↓'])
  const step = dialog.locator('li', { hasText: /previous or next note in time/ })
  await expect(step.locator('kbd')).toHaveText(['←', '→', '↑', '↓', 'Shift', '←', 'Shift', '→', 'Shift', '↑', 'Shift', '↓'])
  await expect(dialog.locator('li', { hasText: /Move the selected notes by the drawing step/ }).locator('kbd')).toHaveText(['Alt', '←', 'Alt', '→'])
  expect(seen.errors).toEqual([])
})

// #527: a drawn note takes the draw velocity of the toolbar, whatever velocity the notes touched before had.
test('piano roll: a drawn note takes the draw velocity, not the velocity of the last touched note', async ({ page }) => {
  test.setTimeout(120_000)
  const { seen } = await openRoll(page)
  const draw = page.getByTestId('roll-draw-velocity')
  await expect(draw).toHaveValue('100')
  const g = await spot(page)

  await page.mouse.click(g.x(100), g.y(60))
  await page.getByTestId('roll-tool-select').click()
  await page.mouse.click(g.x(100), g.y(60))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  // The selection goes down to 1 (Page Down, again and again, to the floor); the draw velocity stays.
  for (let i = 0; i < 12; i++) await page.keyboard.press('PageDown')
  expect((await notes(page)).map((n) => n.velocity)).toEqual([1])
  await expect(draw).toHaveValue('100')

  await page.getByTestId('roll-tool-draw').click()
  await page.mouse.click(g.x(1000), g.y(62))
  expect((await notes(page)).find((n) => n.pitch === 62)!.velocity).toBe(100)

  // The draw velocity is its own setting: typed, kept between 1 and 127, used by the next note.
  await draw.fill('64')
  await draw.blur()
  await page.mouse.click(g.x(2000), g.y(64))
  expect((await notes(page)).find((n) => n.pitch === 64)!.velocity).toBe(64)
  await draw.fill('500')
  await draw.blur()
  await expect(draw).toHaveValue('127')
  expect(seen.errors).toEqual([])
})

// #550: Alt+Left and Alt+Right move the selection from the grid and the free cursor, a refused transposition is said,
// and the draw velocity survives closing the roll.
test('piano roll: Alt+arrows move from the grid and the cursor, an edge refusal is said, the draw velocity is kept', async ({ page }) => {
  test.setTimeout(120_000)
  const { seen } = await openRoll(page)
  const g = await spot(page)
  await page.mouse.click(g.x(1000), g.y(60))
  await focusGrid(page)
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  const before = await starts(page)

  // From the grid: right then left, one entry each.
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(async () => (await starts(page))[0]! - before[0]!).toBeGreaterThan(0)
  await page.keyboard.press('Alt+ArrowLeft')
  await expect.poll(() => starts(page)).toEqual(before)
  // From the free cursor.
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('roll-cursor')).toBeFocused()
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(async () => (await starts(page))[0]! - before[0]!).toBeGreaterThan(0)
  await page.keyboard.press('Alt+ArrowLeft')
  await expect.poll(() => starts(page)).toEqual(before)

  // Up to the keyboard's edge by octaves (60 to 120): the next octave would leave it, so it is refused and said.
  for (let i = 0; i < 5; i++) await page.keyboard.press('Alt+Shift+ArrowUp')
  await expect.poll(() => pitches(page)).toEqual([120])
  const said = page.getByTestId('roll-said')
  const n = Number(await said.getAttribute('data-n'))
  await page.keyboard.press('Alt+Shift+ArrowUp')
  await expect(said).toHaveText(/Cannot move further/)
  expect(Number(await said.getAttribute('data-n'))).toBeGreaterThan(n)
  expect(await pitches(page)).toEqual([120])

  // A key event whose target is the window itself (no node) is ignored, not an error (#550).
  await page.evaluate(() => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true })))
  await page.waitForTimeout(200)
  expect(seen.errors).toEqual([])

  // The draw velocity comes back when the roll is closed and opened again.
  const draw = page.getByTestId('roll-draw-velocity')
  await draw.fill('64')
  await draw.blur()
  await page.getByTestId('panel-devices').click()
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('roll-draw-velocity')).toHaveValue('64')
  expect(seen.errors).toEqual([])
})

// #623: the Length field gives drawn and written notes their length whatever the snap, and Alt+Shift+Left and Right
// change the length of the focused note or of the selection from a note's cell: by one snap step, never under one step,
// cut at the clip's end with a message; one history entry each.
test('piano roll: the Length field sets the written length whatever the snap, and Alt+Shift+arrows resize from a note, at least a step, cut at the clip end', async ({
  page,
}) => {
  test.setTimeout(120_000)
  const { seen } = await openRoll(page)
  await expect(page.getByTestId('roll-length')).toHaveValue('1')
  const g = await spot(page)
  const field = page.getByTestId('roll-draw-length')
  const shape = async () => (await notes(page)).map((n) => [n.start, n.pitch, n.length])
  const said = page.getByTestId('roll-said')
  const focused = page.locator(':focus')

  // A half note at the start of the bar with the snap on a beat, then the snap changes and the field stays.
  await page.getByTestId('roll-snap').selectOption('beat')
  await field.selectOption('half')
  await page.mouse.click(g.x(10), g.y(60))
  await expect.poll(shape).toEqual([[0, 60, 1920]])
  await page.getByTestId('roll-snap').selectOption('quarter-beat')
  await expect(field).toHaveValue('half')
  await page.mouse.click(g.x(1930), g.y(62))
  await expect.poll(shape).toEqual([
    [0, 60, 1920],
    [1920, 62, 1920],
  ])

  // From the first note's cell: one snap step (a sixteenth) longer, said, the label follows, the focus stays; one undo.
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Alt+Shift+ArrowRight')
  await expect.poll(shape).toEqual([
    [0, 60, 2160],
    [1920, 62, 1920],
  ])
  await expect(said).toContainText('Length 2.25 beats')
  await expect(focused).toHaveAttribute('aria-label', /^C4, bar 1 beat 1, 2\.25 beats/)
  await page.keyboard.press('Control+z')
  await expect.poll(shape).toEqual([
    [0, 60, 1920],
    [1920, 62, 1920],
  ])

  // Shorter by beats down to one beat, never under it: the next press is refused and said.
  await page.getByTestId('roll-snap').selectOption('beat')
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Alt+Shift+ArrowLeft')
  await expect.poll(shape).toEqual([
    [0, 60, 960],
    [1920, 62, 1920],
  ])
  await expect(focused).toHaveAttribute('aria-label', /, 1 beat,/)
  await page.keyboard.press('Alt+Shift+ArrowLeft')
  await expect(said).toContainText('Cannot resize further')
  expect((await shape())[0]).toEqual([0, 60, 960])

  // The whole selection longer: the note that ends the clip stays, the other grows, and the cut is said.
  await focusGrid(page)
  await page.keyboard.press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '2')
  await page.keyboard.press('Alt+Shift+ArrowRight')
  await expect.poll(shape).toEqual([
    [0, 60, 1920],
    [1920, 62, 1920],
  ])
  await expect(said).toContainText('cut at the end of the clip')
  await page.keyboard.press('Control+z')
  await expect.poll(shape).toEqual([
    [0, 60, 960],
    [1920, 62, 1920],
  ])
  // The note at the clip's end alone: refused, and said.
  await page.keyboard.press('Escape')
  await page.getByTestId('roll-item').last().focus()
  await page.keyboard.press('Alt+Shift+ArrowRight')
  await expect(said).toContainText('Cannot lengthen past the end of the clip')

  // Enter on the free cursor writes a note of the field's length.
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('roll-cursor')).toBeFocused()
  await page.keyboard.press('ArrowUp')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await shape()).find((n) => n[1] === 61)).toEqual([0, 61, 1920])

  // A chosen length longer than the room left is cut at the clip's end, and said (#280).
  await field.selectOption('four-bars')
  await page.locator('[data-testid="roll-item"][aria-label^="C4,"]').focus()
  await page.keyboard.press('Escape')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await shape()).find((n) => n[1] === 58)).toEqual([0, 58, 3840])
  await expect(said).toContainText('Note added, A#3, Length 1 bar, cut at the end of the clip')
  // From the free cursor, Alt+Shift+Left shortens the selection (the note just written) by one step.
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('roll-cursor')).toBeFocused()
  await page.keyboard.press('Alt+Shift+ArrowLeft')
  await expect.poll(async () => (await shape()).find((n) => n[1] === 58)).toEqual([0, 58, 2880])
  await expect(page.getByTestId('roll-cursor')).toBeFocused()

  // The field comes back when the roll is closed and opened again.
  await page.getByTestId('panel-devices').click()
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('roll-draw-length')).toHaveValue('four-bars')
  expect(seen.errors).toEqual([])
})

// #711: the snap's triplets. The cursor steps by a third of a beat (bar 1 beat 1.33, 1.67), Enter writes a note of that
// length, Alt+Left and Right move it by that step, a click draws on it, and a sixteenth triplet halves the step.
test('piano roll: a triplet snap steps the cursor, writes, moves and draws on triplet positions', async ({ page }) => {
  test.setTimeout(120_000)
  const { seen } = await openRoll(page)
  const snap = page.getByTestId('roll-snap')
  await expect(snap.locator('option').nth(-2)).toHaveText('1/8 triplet')
  await expect(snap.locator('option').last()).toHaveText('1/16 triplet')
  await snap.selectOption('eighth-triplet')
  const shape = async () => (await notes(page)).map((n) => [n.start, n.pitch, n.length])
  const cursor = page.getByTestId('roll-cursor')
  const focused = page.locator(':focus')

  await focusGrid(page)
  await page.keyboard.press('Tab')
  await expect(cursor).toBeFocused()
  await expect(cursor).toHaveAccessibleName(/bar 1 beat 1$/)
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/bar 1 beat 1\.33$/)
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/bar 1 beat 1\.67$/)
  const pitch = Number(await cursor.getAttribute('data-pitch'))
  await page.keyboard.press('Enter')
  await expect.poll(shape).toEqual([[640, pitch, 320]])
  await expect(focused).toHaveAttribute('aria-label', /, bar 1 beat 1\.67,/)

  // Alt+Right and Left move it by the triplet step.
  await page.keyboard.press('Alt+ArrowRight')
  await expect.poll(shape).toEqual([[960, pitch, 320]])
  await page.keyboard.press('Alt+ArrowLeft')
  await page.keyboard.press('Alt+ArrowLeft')
  await expect.poll(shape).toEqual([[320, pitch, 320]])

  // A click draws on the triplet line under the pointer, one triplet long.
  const g = await spot(page)
  await page.mouse.click(g.x(1300), g.y(62))
  await expect.poll(shape).toEqual([
    [320, pitch, 320],
    [1280, 62, 320],
  ])

  // A sixteenth triplet: the cursor steps by a sixth of a beat.
  await snap.selectOption('sixteenth-triplet')
  await page.getByTestId('roll-item').first().focus()
  await page.keyboard.press('Escape')
  await expect(cursor).toBeFocused()
  await page.keyboard.press('ArrowRight')
  await expect(cursor).toHaveAccessibleName(/bar 1 beat 1\.5$/)

  // The Length field has the triplets too (#623): an eighth triplet written on a sixteenth-triplet position.
  const length = page.getByTestId('roll-draw-length')
  await expect(length.locator('option[value="sixteenth-triplet"]')).toHaveText('1/16 triplet')
  await length.selectOption('eighth-triplet')
  await cursor.focus()
  await page.keyboard.press('ArrowUp')
  await page.keyboard.press('Enter')
  await expect.poll(shape).toEqual([
    [320, pitch, 320],
    [480, pitch + 1, 320],
    [1280, 62, 320],
  ])
  expect(seen.errors).toEqual([])
})
