import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #615: the arrangement's Draw and Select tools, with the mouse at coordinates in a real browser. In Draw, a drag on the free
// ground of a MIDI track makes a clip of the dragged length (rightwards or leftwards, on the snap, Ctrl frees it) in one command,
// so one undo; in Select the same drag draws a box that selects the clips it touches; a double click creates a clip in both, and
// grabbing a clip moves or resizes it in both.

const RULER = 28
/** The height of a track's row: the `lane` token (#471). */
const LANE = 44
const BEAT = 960
const BAR = 3840

interface Block {
  id: string
  row: number
  start: number
  length: number
}

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const arr = (page: Page) => page.getByTestId('arrangement')
const stage = (page: Page) => page.getByTestId('arr-stage')
const blocks = async (page: Page): Promise<Block[]> =>
  (JSON.parse((await arr(page).getAttribute('data-blocks')) ?? '[]') as Block[]).sort((a, b) => a.start - b.start || (a.id < b.id ? -1 : 1))
const spans = async (page: Page) => (await blocks(page)).map((b) => [b.start, b.length])
const selected = async (page: Page): Promise<string[]> => JSON.parse((await arr(page).getAttribute('data-selection')) ?? '[]')

async function spot(page: Page) {
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await stage(page).getAttribute('data-scroll-y'))
  return {
    x: (tick: number) => box.x + tick * ppt,
    y: (row: number) => box.y + RULER + row * LANE - scrollY + LANE / 2,
    ppt,
  }
}

async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }, keys: string[] = []) {
  for (const key of keys) await page.keyboard.down(key)
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 4 })
  await page.mouse.move(to.x, to.y, { steps: 4 })
  await page.mouse.up()
  for (const key of keys) await page.keyboard.up(key)
}

const undo = async (page: Page) => {
  await stage(page).getByTestId('timegrid').focus()
  await page.keyboard.press('Control+z')
}

test('arrangement: Draw makes a clip of the dragged length, Select draws a box, a double click and the grab work in both', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  const refused: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'warning' && /refused/.test(message.text())) refused.push(message.text())
  })
  await page.setViewportSize({ width: 1800, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await engineUp(page)
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toHaveAttribute('data-block-count', '0')
  await expect.poll(async () => Number(await stage(page).getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await page.getByTestId('arr-snap').selectOption('beat')
  const g = await spot(page)
  const tolerance = Math.ceil(2 / g.ppt)

  // Select is the tool at first, and in it a drag on the free ground draws nothing but a box.
  await expect(arr(page)).toHaveAttribute('data-mode', 'select')
  await expect(page.getByTestId('arr-tool-select')).toHaveAttribute('aria-checked', 'true')
  await drag(page, { x: g.x(2000), y: g.y(0) }, { x: g.x(6000), y: g.y(0) })
  expect(await blocks(page)).toEqual([])

  // Draw rightwards: the clip follows the pointer, both ends on the beat; the label shows the length while the button is down.
  await page.getByTestId('arr-tool-draw').click()
  await expect(arr(page)).toHaveAttribute('data-mode', 'draw')
  await page.mouse.move(g.x(2000), g.y(0))
  await page.mouse.down()
  await page.mouse.move(g.x(4000), g.y(0), { steps: 4 })
  await page.mouse.move(g.x(6000), g.y(0), { steps: 4 })
  await expect(page.getByTestId('gesture-label')).toContainText(/Length|Longueur/)
  await expect(page.getByTestId('gesture-label')).toContainText(/Snap|Magn/)
  expect(await blocks(page)).toEqual([])
  await page.mouse.up()
  // 2000 and 6000 snap to 1920 and 5760.
  expect(await spans(page)).toEqual([[2 * BEAT, 4 * BEAT]])
  await expect(page.getByTestId('gesture-label')).toHaveCount(0)
  // The new clip is the selection.
  expect(await selected(page)).toHaveLength(1)

  // One undo takes the whole gesture back.
  await undo(page)
  await expect(arr(page)).toHaveAttribute('data-block-count', '0')

  // Draw leftwards: from the right to the left, the same kind of clip.
  await drag(page, { x: g.x(11500), y: g.y(0) }, { x: g.x(8000), y: g.y(0) })
  expect(await spans(page)).toEqual([[8 * BEAT, 4 * BEAT]])
  await undo(page)
  await expect(arr(page)).toHaveAttribute('data-block-count', '0')

  // Ctrl sets the snap aside: the clip has the length that was dragged, to the pixel.
  await drag(page, { x: g.x(2000), y: g.y(0) }, { x: g.x(3500), y: g.y(0) }, ['Control'])
  const free = await blocks(page)
  expect(free).toHaveLength(1)
  expect(Math.abs(free[0]!.start - 2000)).toBeLessThanOrEqual(tolerance)
  expect(Math.abs(free[0]!.length - 1500)).toBeLessThanOrEqual(2 * tolerance)
  expect(free[0]!.start % BEAT).not.toBe(0)
  await undo(page)
  await expect(arr(page)).toHaveAttribute('data-block-count', '0')

  // Escape ends a drawing without a clip; a plain click draws none either.
  await page.mouse.move(g.x(2000), g.y(0))
  await page.mouse.down()
  await page.mouse.move(g.x(5000), g.y(0), { steps: 4 })
  await page.keyboard.press('Escape')
  await page.mouse.up()
  await page.mouse.click(g.x(2000), g.y(0))
  expect(await blocks(page)).toEqual([])

  // A drawing stops at the neighbouring clip: it never overlaps.
  await drag(page, { x: g.x(5000), y: g.y(0) }, { x: g.x(6500), y: g.y(0) })
  expect(await spans(page)).toEqual([[5 * BEAT, 2 * BEAT]])
  await drag(page, { x: g.x(2000), y: g.y(0) }, { x: g.x(9000), y: g.y(0) })
  expect(await spans(page)).toEqual([
    [2 * BEAT, 3 * BEAT],
    [5 * BEAT, 2 * BEAT],
  ])

  // A double click creates a clip of a bar in Draw...
  await page.mouse.dblclick(g.x(BAR * 4 + 100), g.y(0))
  await expect(arr(page)).toHaveAttribute('data-block-count', '3')
  expect((await blocks(page)).map((b) => [b.start, b.length])).toContainEqual([BAR * 4, BAR])

  // ...and grabbing a clip moves it in Draw (the snap on the beat)...
  await drag(page, { x: g.x(6000), y: g.y(0) }, { x: g.x(6000 + 2 * BEAT), y: g.y(0) })
  expect(await spans(page)).toContainEqual([7 * BEAT, 2 * BEAT])

  // ...and, in Select, resizes by its edge, creates by double click and draws the box.
  await page.getByTestId('arr-tool-select').click()
  const second = (await blocks(page)).find((b) => b.start === 7 * BEAT)!
  await drag(page, { x: g.x(second.start + second.length) - 3, y: g.y(0) }, { x: g.x(second.start + 3 * BEAT) - 3, y: g.y(0) })
  expect(await spans(page)).toContainEqual([7 * BEAT, 3 * BEAT])
  await page.mouse.dblclick(g.x(BAR * 8 + 100), g.y(0))
  await expect(arr(page)).toHaveAttribute('data-block-count', '4')

  // A box from the free ground selects the clips it touches, whichever corner it starts from.
  await drag(page, { x: g.x(BAR * 3), y: g.y(0) + 12 }, { x: g.x(BAR - 100), y: g.y(0) - 12 })
  const all = await blocks(page)
  expect((await selected(page)).sort()).toEqual(
    all
      .filter((b) => b.start < BAR * 4)
      .map((b) => b.id)
      .sort(),
  )
  expect(await selected(page)).toHaveLength(2)
  // A click on the free ground clears it.
  await page.mouse.click(g.x(BAR * 12), g.y(0))
  expect(await selected(page)).toEqual([])
  // Nothing of the box was sent to the project.
  expect(await blocks(page)).toEqual(all)

  expect(refused).toEqual([])
  expect(seen.errors).toEqual([])
})
