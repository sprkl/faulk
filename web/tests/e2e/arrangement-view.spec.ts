import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// AR-4 (#80): the arrangement view in a real browser, with the real worklet, engine and project core. Every
// gesture reaches the project as an arrangement command (a refused command would be a console warning, which
// the test forbids), and a drag is one history entry: one undo takes it back. The placements are read from the
// view's data attributes (the project store's view).

const RULER = 28
const LANE = 64
const BAR = 3840

interface Block {
  id: string
  track: string
  row: number
  clip: string
  start: number
  length: number
  offset: number
}

const startAudio = (page: Page) => page.getByTestId('engine-state').getByRole('button', { name: /Start audio|Démarrer/ }).click()
const arr = (page: Page) => page.getByTestId('arrangement')
const stage = (page: Page) => page.getByTestId('arr-stage')
const blocks = async (page: Page): Promise<Block[]> =>
  (JSON.parse((await arr(page).getAttribute('data-blocks')) ?? '[]') as Block[]).sort((a, b) => a.start - b.start || (a.id < b.id ? -1 : 1))
const selected = async (page: Page): Promise<string[]> => JSON.parse((await arr(page).getAttribute('data-selection')) ?? '[]')

/** Where a tick and a lane are on the page. */
async function spot(page: Page) {
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await stage(page).getAttribute('data-scroll-y'))
  return {
    x: (tick: number) => box.x + tick * ppt,
    y: (row: number) => box.y + RULER + row * LANE - scrollY + LANE / 2,
    ruler: box.y + RULER / 2,
  }
}

async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }, options: { alt?: boolean; shift?: boolean } = {}) {
  if (options.alt) await page.keyboard.down('Alt')
  if (options.shift) await page.keyboard.down('Shift')
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 4 })
  await page.mouse.move(to.x, to.y, { steps: 4 })
  await page.mouse.up()
  if (options.shift) await page.keyboard.up('Shift')
  if (options.alt) await page.keyboard.up('Alt')
}

test('arrangement view: place, move, resize, split, duplicate, delete, loop, seek, each gesture one command', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  const refused: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'warning' && /refused/.test(message.text())) refused.push(message.text())
  })
  await page.setViewportSize({ width: 1800, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()

  // A one-bar clip at the start of the selected track, made in the piano roll.
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '0')
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toHaveAttribute('data-block-count', '1')
  await expect.poll(async () => Number(await stage(page).getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await stage(page).getByTestId('timegrid').focus()
  await page.getByTestId('arr-snap').selectOption('beat')
  const g = await spot(page)
  const [first] = await blocks(page)
  expect(first).toMatchObject({ start: 0, length: BAR, offset: 0 })

  // Select, then duplicate: an independent copy right after, on a clip of its own.
  await page.mouse.click(g.x(1000), g.y(0))
  expect(await selected(page)).toEqual([first!.id])
  await page.getByTestId('arr-duplicate').click()
  await expect(arr(page)).toHaveAttribute('data-block-count', '2')
  let all = await blocks(page)
  expect(all[1]).toMatchObject({ start: BAR, length: BAR })
  expect(all[1]!.clip).not.toBe(all[0]!.clip)
  expect(await selected(page)).toEqual([all[1]!.id])

  // Duplicate linked (Ctrl+Shift+D): the copy plays the same clip.
  await stage(page).getByTestId('timegrid').focus()
  await page.keyboard.press('Control+Shift+D')
  await expect(arr(page)).toHaveAttribute('data-block-count', '3')
  all = await blocks(page)
  expect(all[2]).toMatchObject({ start: 2 * BAR, clip: all[1]!.clip })

  // A drag is one command: the last block moves one bar later (by Alt-less drag), and one undo brings it back.
  const third = all[2]!
  await drag(page, { x: g.x(third.start + 1500), y: g.y(0) }, { x: g.x(third.start + 1500 + BAR), y: g.y(0) })
  await expect.poll(async () => (await blocks(page)).map((b) => b.start)).toEqual([0, BAR, 3 * BAR])
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await blocks(page)).map((b) => b.start)).toEqual([0, BAR, 2 * BAR])

  // A plain click moves nothing and a drag on the empty lane selects nothing.
  await page.mouse.click(g.x(1000), g.y(0))
  expect((await blocks(page)).map((b) => b.start)).toEqual([0, BAR, 2 * BAR])

  // Resize the right edge of the first block to two beats.
  await drag(page, { x: g.x(BAR) - 2, y: g.y(0) }, { x: g.x(1920), y: g.y(0) })
  await expect.poll(async () => (await blocks(page))[0]!.length).toBe(1920)
  expect((await blocks(page))[1]).toMatchObject({ start: BAR, length: BAR })
  // Its start edge, to the next beat: the clip is shown from further on.
  await drag(page, { x: g.x(0) + 2, y: g.y(0) }, { x: g.x(960), y: g.y(0) })
  await expect.poll(async () => (await blocks(page))[0]).toMatchObject({ start: 960, length: 960, offset: 960 })

  // Seek by a click on the ruler, then split the selection at the play position.
  await page.getByTestId('arr-snap').selectOption('half-beat')
  await page.mouse.click(g.x(1440), g.ruler)
  await expect(arr(page)).toHaveAttribute('data-position', '1440')
  await page.mouse.click(g.x(1200), g.y(0))
  await page.keyboard.press('Control+e')
  await expect(arr(page)).toHaveAttribute('data-block-count', '4')
  all = await blocks(page)
  expect(all.slice(0, 2)).toMatchObject([
    { start: 960, length: 480 },
    { start: 1440, length: 480 },
  ])

  // Delete the selection.
  await page.keyboard.press('Delete')
  await expect(arr(page)).toHaveAttribute('data-block-count', '3')

  // Alt+drag copies where it is dropped: the originals stay.
  const before = await blocks(page)
  await drag(page, { x: g.x(BAR + 1000), y: g.y(0) }, { x: g.x(BAR + 1000 + 3 * BAR), y: g.y(0) }, { alt: true })
  await expect(arr(page)).toHaveAttribute('data-block-count', '4')
  expect((await blocks(page)).slice(0, 3).map((b) => b.id)).toEqual(before.map((b) => b.id))
  // An Alt+click without a drag copies nothing.
  await page.keyboard.down('Alt')
  await page.mouse.click(g.x(BAR + 1000), g.y(0))
  await page.keyboard.up('Alt')
  await expect(arr(page)).toHaveAttribute('data-block-count', '4')

  // The loop region, by a drag on the ruler: the region is set and the loop is on, in one undo.
  await page.getByTestId('arr-snap').selectOption('bar')
  await drag(page, { x: g.x(BAR), y: g.ruler }, { x: g.x(3 * BAR), y: g.ruler })
  await expect(arr(page)).toHaveAttribute('data-loop', `${BAR}-${3 * BAR}`)
  await expect(arr(page)).toHaveAttribute('data-loop-on', 'true')
  await page.keyboard.press('Control+z')
  await expect(arr(page)).toHaveAttribute('data-loop-on', 'false')

  // A double-click on an empty place of the lane makes a one-bar clip; on a clip it opens the piano roll.
  await page.mouse.dblclick(g.x(3 * BAR + 100), g.y(0))
  await expect(arr(page)).toHaveAttribute('data-block-count', '5')
  await page.getByTestId('panel-devices').click()
  await page.mouse.dblclick(g.x(3 * BAR + 1000), g.y(0))
  await expect(page.getByTestId('piano-roll')).toBeVisible()

  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/arrangement-view.png` })
  expect(refused).toEqual([])
  expect(seen.errors).toEqual([])
})
