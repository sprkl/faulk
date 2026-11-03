import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #524: in the app the right click never opens the browser's menu. A target with actions opens the shared menu (note,
// clip, track head, cell, device), anything else opens nothing, and a field where text is typed keeps the browser's own
// menu. The menu is a keyboard menu: Shift+F10 or the Menu key open it, the arrows move, Enter picks, Escape closes.

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
const menu = (page: Page) => page.getByTestId('context-menu')
const notes = async (page: Page): Promise<Note[]> => JSON.parse((await roll(page).getAttribute('data-notes')) ?? '[]')
const num = async (page: Page, testid: string, attribute: string) => Number(await page.getByTestId(testid).getAttribute(attribute))

async function spot(page: Page) {
  const box = (await stage(page).boundingBox())!
  const ppt = await num(page, 'timegrid', 'data-px-per-tick')
  const scrollY = await num(page, 'roll-stage', 'data-scroll-y')
  return { x: (tick: number) => box.x + tick * ppt, y: (pitch: number) => box.y + RULER + (127 - pitch) * ROW - scrollY + ROW / 2 }
}

async function openRoll(page: Page, lang?: 'fr') {
  const seen = watch(page)
  if (lang) await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width: 1400, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  return seen
}

/** Sends a `contextmenu` event to an element and tells whether the page cancelled it (the browser's menu would not open). */
const sendContextMenu = (page: Page, selector: string) =>
  page.evaluate((sel) => {
    const element = document.querySelector(sel)!
    const box = element.getBoundingClientRect()
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, composed: true, clientX: box.left + 4, clientY: box.top + 4, button: 2 })
    element.dispatchEvent(event)
    return event.defaultPrevented
  }, selector)

test('the browser menu never opens: nothing on a background, the browser menu kept in text fields', async ({ page }) => {
  const seen = await openRoll(page)

  // A background, the top bar, the track column, a toolbar button: cancelled, and no menu.
  for (const selector of ['body', '[data-testid="engine-state"]', '[data-testid="view"]', '[data-testid="roll-snap"]', '[data-testid="roll-quantize"]']) {
    expect(await sendContextMenu(page, selector), selector).toBe(true)
    await expect(menu(page)).toHaveCount(0)
  }
  // The real right click on the page's top bar does the same.
  const bar = (await page.getByTestId('engine-state').boundingBox())!
  await page.mouse.click(bar.x + 2, bar.y + 2, { button: 'right' })
  await expect(menu(page)).toHaveCount(0)

  // The fields where text is typed keep the browser's menu: the event is not cancelled.
  expect(await sendContextMenu(page, '[data-testid="roll-quantize-strength"]')).toBe(false)
  await page.getByTestId('roll-clip-name').dblclick()
  const name = page.locator('input[aria-label="Clip name"], input[aria-label="Nom du clip"]')
  await expect(name).toBeVisible()
  expect(await sendContextMenu(page, 'input[maxlength]')).toBe(false)
  expect(seen.errors).toEqual([])
})

test('piano roll: the right click on a note opens its actions, the keyboard reaches them, quantize is one history entry', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = await openRoll(page)
  await page.getByTestId('roll-snap').selectOption('off')
  const g = await spot(page)
  await page.mouse.click(g.x(130), g.y(60))
  await page.mouse.click(g.x(1100), g.y(62))
  const drawn = await notes(page)
  expect(drawn).toHaveLength(2)

  // The empty grid of a clip that has notes: only what needs no selection (quantize all), nothing to paste.
  await page.keyboard.press('Escape')
  await page.mouse.click(g.x(3000), g.y(57), { button: 'right' })
  await expect(menu(page)).toHaveCount(1)
  await expect(page.getByTestId('ctx-quantize')).toBeVisible()
  await expect(page.getByTestId('ctx-duplicate')).toHaveCount(0)
  await page.keyboard.press('Escape')
  await expect(menu(page)).toHaveCount(0)
  // The ruler has no action.
  const box = (await stage(page).boundingBox())!
  await page.mouse.click(box.x + 100, box.y + 8, { button: 'right' })
  await expect(menu(page)).toHaveCount(0)

  // A note: it joins the selection and the menu opens with its actions and their shortcuts.
  await page.mouse.click(g.x(130 + 20), g.y(60), { button: 'right' })
  await expect(menu(page)).toBeVisible()
  await expect(menu(page)).toHaveAttribute('aria-label', /Note actions|Actions sur les notes/)
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  for (const id of ['quantize', 'duplicate', 'copy', 'cut', 'delete']) await expect(page.getByTestId(`ctx-${id}`)).toBeVisible()
  await expect(page.getByTestId('ctx-duplicate')).toContainText('Ctrl+D')
  await expect(page.getByTestId('ctx-delete')).toContainText('Delete')
  // The first entry has the focus; a click elsewhere closes the menu.
  await expect(page.getByTestId('ctx-quantize')).toBeFocused()
  await page.getByTestId('roll-count').click()
  await expect(menu(page)).toHaveCount(0)

  // Quantize from the menu: the chosen note moves to the grid, in one history entry, and one undo brings it back.
  await page.mouse.click(g.x(130 + 20), g.y(60), { button: 'right' })
  await page.getByTestId('ctx-quantize').click()
  await expect(menu(page)).toHaveCount(0)
  // The chosen note (130) goes to the 1/16 line at 240; the other one (1100) is not chosen and stays.
  await expect.poll(async () => (await notes(page)).map((n) => n.start).sort((a, b) => a - b)).toEqual([240, 1100])
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await notes(page)).map((n) => n.start).sort((a, b) => a - b)).toEqual(drawn.map((n) => n.start).sort((a, b) => a - b))

  // From the keyboard: a note's cell has the focus, Shift+F10 opens the menu on it, the arrows move, Enter picks, Escape closes.
  const cell = page.locator('[data-testid="roll-items"] [data-note]').first()
  await cell.focus()
  await page.keyboard.press('Shift+F10')
  await expect(menu(page)).toBeVisible()
  await expect(page.getByTestId('ctx-quantize')).toBeFocused()
  await page.keyboard.press('ArrowDown')
  await expect(page.getByTestId('ctx-duplicate')).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(menu(page)).toHaveCount(0)
  await expect(cell).toBeFocused()
  // The Menu key is the same; Enter on the entry sends its command (duplicate: the selection is copied a bar later).
  await page.keyboard.press('ContextMenu')
  await expect(menu(page)).toBeVisible()
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(menu(page)).toHaveCount(0)
  await expect(roll(page)).toHaveAttribute('data-note-count', '3')

  expect(seen.errors).toEqual([])
})

test('the menu is in French when the page is', async ({ page }) => {
  const seen = await openRoll(page, 'fr')
  await page.getByTestId('roll-snap').selectOption('off')
  const g = await spot(page)
  await page.mouse.click(g.x(130), g.y(60))
  await page.mouse.click(g.x(130 + 20), g.y(60), { button: 'right' })
  await expect(page.getByTestId('ctx-quantize')).toHaveText(/Quantifier/)
  await expect(page.getByTestId('ctx-copy')).toHaveText(/Copier/)
  await expect(page.getByTestId('ctx-cut')).toHaveText(/Couper/)
  await expect(page.getByTestId('ctx-delete')).toHaveText(/Supprimer/)
  expect(seen.errors).toEqual([])
})

test('arrangement: a clip, the free part of a row and a track head have their actions; the ruler has none', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = await openRoll(page)
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '1')
  const arr = page.getByTestId('arr-stage')
  await expect.poll(async () => Number(await arr.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  const box = (await arr.boundingBox())!
  const ppt = Number(await arr.getByTestId('timegrid').getAttribute('data-px-per-tick'))

  // The ruler: nothing.
  await page.mouse.click(box.x + 60, box.y + 8, { button: 'right' })
  await expect(menu(page)).toHaveCount(0)

  // The free part of the MIDI row: creating a clip there.
  await page.mouse.click(box.x + 1.5 * 3840 * ppt, box.y + RULER + 22, { button: 'right' })
  await expect(page.getByTestId('ctx-create')).toBeVisible()
  await page.getByTestId('ctx-create').click()
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '2')

  // The clip: open, duplicate, linked, split, delete; Delete sends the command.
  await page.mouse.click(box.x + (3840 + 1000) * ppt, box.y + RULER + 22, { button: 'right' })
  await expect(menu(page)).toBeVisible()
  for (const id of ['open', 'duplicate', 'duplicate-linked', 'split', 'delete']) await expect(page.getByTestId(`ctx-${id}`)).toBeVisible()
  await page.getByTestId('ctx-duplicate').click()
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '3')

  // The track head: rename, duplicate, colour (a second list in place of the first), delete.
  const head = page.getByTestId('track-header').first()
  await head.click({ button: 'right', position: { x: 40, y: 10 } })
  await expect(menu(page)).toBeVisible()
  for (const id of ['rename', 'duplicate', 'color', 'delete']) await expect(page.getByTestId(`ctx-${id}`)).toBeVisible()
  await page.getByTestId('ctx-color').click()
  await expect(menu(page)).toHaveAttribute('aria-label', /Track colour|Couleur de la piste/)
  await expect(menu(page).getByRole('menuitem')).toHaveCount(16)
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(menu(page)).toHaveCount(0)
  const colored = await head.evaluate((el) => getComputedStyle(el).getPropertyValue('--swatch').trim())
  expect(colored).toMatch(/^#[0-9a-f]{6}$/)
  await head.click({ button: 'right', position: { x: 40, y: 10 } })
  await page.getByTestId('ctx-duplicate').click()
  await expect(page.getByTestId('track-header')).toHaveCount(2)

  // A name being renamed keeps the browser's menu.
  await head.click({ button: 'right', position: { x: 40, y: 10 } })
  await page.getByTestId('ctx-rename').click()
  const field = head.locator('input.rename')
  await expect(field).toBeVisible()
  await expect(field).toBeFocused()
  expect(await sendContextMenu(page, '[data-testid="track-header"] input.rename')).toBe(false)
  await page.keyboard.press('Escape')

  expect(seen.errors).toEqual([])
})

test('devices: the card of a device has its actions', async ({ page }) => {
  const seen = await openRoll(page)
  await page.getByTestId('panel-devices').click()
  const card = page.getByTestId('device-card').first()
  await expect(card).toBeVisible()
  // On the card's title, not on a knob: a knob answers for itself.
  await card.locator('h3').click({ button: 'right' })
  await expect(menu(page)).toBeVisible()
  await expect(page.getByTestId('ctx-bypass')).toBeVisible()
  await page.getByTestId('ctx-bypass').click()
  await expect(card).toHaveAttribute('data-bypassed', 'true')
  expect(seen.errors).toEqual([])
})
