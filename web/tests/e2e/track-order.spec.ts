import { expect, test, type Locator, type Page } from '@playwright/test'
import { watch } from './support'

// #298: reorder and duplicate a track from the track list (since #471: the arrangement's rows and the inspector), in a real browser. Real worklet, engine and
// project core: the `⋯` menu and the drag of the grip each send one command (`track.move`,
// `track.duplicate`), so one undo takes a gesture back. The order is read in the track list, the
// launch grid, the mixer and the arrangement view: it is the same everywhere.

// The tracks are the arrangement's rows (#471), and the actions menu is in the inspector of the selected track:
// these tests start on the arrangement, the app's default view.
test.use({ storageState: { cookies: [], origins: [] } })

const headers = (page: Page) => page.getByTestId('track-header')
const header = (page: Page, name: string) => headers(page).filter({ has: page.getByTestId('track-name').getByText(name, { exact: true }) })
const listOrder = (page: Page) => page.getByTestId('track-name').allInnerTexts()
const undo = (page: Page) => page.getByRole('button', { name: 'Undo', exact: true })
const redo = (page: Page) => page.getByRole('button', { name: 'Redo', exact: true })

/** Selects the track by its row, then opens the inspector's actions menu. */
async function openMenu(page: Page, name: string) {
  await header(page, name).getByTestId('arr-track').click()
  await expect(page.getByTestId('inspector-name')).toHaveText(name)
  await page.getByTestId('track-inspector').getByTestId('track-more').click()
}

async function menu(page: Page, name: string, item: string) {
  await openMenu(page, name)
  await page.getByTestId('track-menu').getByRole('menuitem', { name: item, exact: true }).click()
}

async function rename(page: Page, from: string, to: string) {
  await menu(page, from, 'Rename')
  await expect(page.getByRole('textbox', { name: 'Track name', exact: true })).toBeFocused()
  await page.keyboard.type(to)
  await page.keyboard.press('Enter')
  await expect(header(page, to)).toHaveCount(1)
}

/** The text content, not the rendered text: a view may set its labels in capitals. */
const text = (locator: Locator) => locator.evaluateAll((els) => els.map((el) => el.textContent?.trim() ?? ''))

/** The track order as each of the four views shows it. */
async function orders(page: Page) {
  const seen: Record<string, string[]> = { list: await listOrder(page) }
  await page.getByTestId('nav-grid').click()
  await expect(page.getByTestId('grid-track')).toHaveCount(seen.list.length)
  seen.grid = await text(page.getByTestId('grid-track'))
  await page.getByTestId('nav-mixer').click()
  await expect(page.getByTestId('mixer-strip')).toHaveCount(seen.list.length)
  seen.mixer = await page.getByTestId('mixer-name').allInnerTexts()
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('arr-track')).toHaveCount(seen.list.length)
  seen.arrangement = await text(page.getByTestId('arr-heads').getByTestId('track-name'))
  return seen
}

async function sameEverywhere(page: Page, expected: string[]) {
  const seen = await orders(page)
  for (const [view, order] of Object.entries(seen)) expect(order, view).toEqual(expected)
}

test('a track moves and duplicates from the track list, one history entry each, in every view', async ({ page }) => {
  const { errors } = watch(page)
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'en'))
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await expect(headers(page)).toHaveCount(1)

  for (const instrument of ['Drums', 'Oscillator']) {
    await page.getByTestId('add-track').click()
    await page.getByRole('menuitem', { name: instrument, exact: true }).click()
  }
  await rename(page, 'Track 2', 'Kit')
  await rename(page, 'Track 3', 'Bass')
  await sameEverywhere(page, ['Synth', 'Kit', 'Bass'])

  // The ends of the list cannot go further.
  await openMenu(page, 'Synth')
  await expect(page.getByTestId('track-menu').getByRole('menuitem', { name: 'Move up', exact: true })).toBeDisabled()
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/track-menu.png` })
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('track-menu')).toHaveCount(0)
  await openMenu(page, 'Bass')
  await expect(page.getByTestId('track-menu').getByRole('menuitem', { name: 'Move down', exact: true })).toBeDisabled()
  await page.keyboard.press('Escape')

  // Move up from the menu: one command, so one undo takes the order back, and a redo puts it again.
  await menu(page, 'Bass', 'Move up')
  await sameEverywhere(page, ['Synth', 'Bass', 'Kit'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Kit', 'Bass'])
  await redo(page).click()
  await sameEverywhere(page, ['Synth', 'Bass', 'Kit'])
  await menu(page, 'Synth', 'Move down')
  await sameEverywhere(page, ['Bass', 'Synth', 'Kit'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Bass', 'Kit'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Kit', 'Bass'])

  // Drag: the grip of a header dropped on another puts it at that place, as one command.
  await header(page, 'Bass').getByTestId('track-grip').dragTo(header(page, 'Synth'))
  expect(await listOrder(page)).toEqual(['Bass', 'Synth', 'Kit'])
  await sameEverywhere(page, ['Bass', 'Synth', 'Kit'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Kit', 'Bass'])
  // Downward: dropped on the last header, the track goes last. Dropped on itself, nothing moves.
  await header(page, 'Synth').getByTestId('track-grip').dragTo(header(page, 'Bass'))
  await sameEverywhere(page, ['Kit', 'Bass', 'Synth'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Kit', 'Bass'])
  await header(page, 'Kit').getByTestId('track-grip').dragTo(header(page, 'Kit'))
  expect(await listOrder(page)).toEqual(['Synth', 'Kit', 'Bass'])

  // Duplicate a muted and armed track: the copy sits right below, under a free name, selected,
  // audible and not armed; the source keeps its state; one undo removes the copy only.
  const kit = header(page, 'Kit')
  await kit.getByRole('button', { name: 'Mute Kit', exact: true }).click()
  await kit.getByTestId('arr-arm').click()
  await expect(kit.getByTestId('arr-arm')).toHaveAttribute('aria-pressed', 'true')
  await menu(page, 'Kit', 'Duplicate')
  await sameEverywhere(page, ['Synth', 'Kit', 'Kit 2', 'Bass'])
  const copy = header(page, 'Kit 2')
  await expect(copy).toHaveAttribute('aria-current', 'true')
  await expect(copy).toContainText('MIDI · Drums')
  await expect(copy.getByRole('button', { name: 'Mute Kit 2', exact: true })).toHaveAttribute('aria-pressed', 'false')
  await expect(copy.getByTestId('arr-arm')).toHaveAttribute('aria-pressed', 'false')
  await expect(kit.getByRole('button', { name: 'Mute Kit', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await expect(kit.getByTestId('arr-arm')).toHaveAttribute('aria-pressed', 'true')
  await menu(page, 'Kit', 'Duplicate')
  await sameEverywhere(page, ['Synth', 'Kit', 'Kit 3', 'Kit 2', 'Bass'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Kit', 'Kit 2', 'Bass'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Kit', 'Bass'])
  await expect(kit.getByRole('button', { name: 'Mute Kit', exact: true })).toHaveAttribute('aria-pressed', 'true')

  // Delete from the menu, and undo.
  await menu(page, 'Bass', 'Delete')
  await sameEverywhere(page, ['Synth', 'Kit'])
  await undo(page).click()
  await sameEverywhere(page, ['Synth', 'Kit', 'Bass'])

  expect(errors).toEqual([])
})

test('the menu of a track speaks French', async ({ page }) => {
  const { errors } = watch(page)
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(headers(page)).toHaveCount(1)
  await page.getByTestId('track-inspector').getByTestId('track-more').click()
  await expect(page.getByTestId('track-menu').getByRole('menuitem')).toHaveText(['Renommer', 'Monter', 'Descendre', 'Dupliquer', 'Supprimer'])
  await page.getByTestId('track-menu').getByRole('menuitem', { name: 'Dupliquer', exact: true }).click()
  await expect(page.getByTestId('track-name')).toHaveText(['Synth', 'Synth 2'])
  expect(errors).toEqual([])
})
