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

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
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
  // One undo took the region back with the switch: the region and the switch were one history group.
  await expect(arr(page)).toHaveAttribute('data-loop', '')

  // Shift-click adds to the selection and takes out; a click under the lanes clears it.
  const now = await blocks(page)
  await page.mouse.click(g.x(now[0]!.start + 100), g.y(0))
  await page.keyboard.down('Shift')
  await page.mouse.click(g.x(now[1]!.start + 100), g.y(0))
  await expect(arr(page)).toHaveAttribute('data-selected-count', '2')
  await page.mouse.click(g.x(now[1]!.start + 100), g.y(0))
  await page.keyboard.up('Shift')
  await expect(arr(page)).toHaveAttribute('data-selected-count', '1')
  await page.mouse.click(g.x(1000), g.y(1))
  await expect(arr(page)).toHaveAttribute('data-selected-count', '0')

  // An Alt+Shift drag copies as linked clips; Escape in the middle of a drag sends nothing.
  const base = (await blocks(page)).length
  await drag(page, { x: g.x(BAR + 1000), y: g.y(0) }, { x: g.x(BAR + 1000 + 8 * BAR), y: g.y(0) }, { alt: true, shift: true })
  await expect(arr(page)).toHaveAttribute('data-block-count', String(base + 1))
  const linked = await blocks(page)
  expect(linked.filter((b) => b.clip === linked.find((c) => c.start === BAR)!.clip).length).toBe(3)
  await page.mouse.move(g.x(BAR + 1000), g.y(0))
  await page.mouse.down()
  await page.mouse.move(g.x(BAR + 2000), g.y(0), { steps: 3 })
  await page.keyboard.press('Escape')
  await page.mouse.up()
  expect((await blocks(page)).map((b) => b.start)).toEqual(linked.map((b) => b.start))

  // A double-click on an empty place of the lane makes a one-bar clip; on a clip it opens the piano roll.
  await page.mouse.dblclick(g.x(3 * BAR + 100), g.y(0))
  await expect(arr(page)).toHaveAttribute('data-block-count', '6')
  await page.getByTestId('panel-devices').click()
  await page.mouse.dblclick(g.x(2 * BAR + 1000), g.y(0))
  await expect(page.getByTestId('piano-roll')).toBeVisible()

  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/arrangement-view.png` })
  expect(refused).toEqual([])
  expect(seen.errors).toEqual([])
})

// #315: the view follows the play position by page, stops at a hand scroll, starts again with Follow or the next Play;
// the scrollbar is visible and the zoom buttons change the visible range.
test('arrangement view: follow the play position, scrollbar, zoom buttons', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('nav-arrangement').click()
  const grid = stage(page).getByTestId('timegrid')
  const num = async (attribute: string) => Number(await grid.getAttribute(attribute))
  const scroll = () => num('data-scroll-tick')
  const play = () => page.getByTestId('transport').getByRole('button', { name: 'Play', exact: true }).click()
  const stop = () => page.getByTestId('transport').getByRole('button', { name: 'Stop', exact: true }).click()
  await expect.poll(() => num('data-draw-count')).toBeGreaterThan(0)

  // The zoom buttons change the visible range (px per tick).
  const ppt0 = await num('data-px-per-tick')
  for (let i = 0; i < 4; i++) await page.getByTestId('arr-zoom-in').click()
  await expect.poll(() => num('data-px-per-tick')).toBeGreaterThan(ppt0 * 2)
  await page.getByTestId('arr-zoom-out').click()
  await expect.poll(() => num('data-px-per-tick')).toBeLessThan(ppt0 * 2.5)

  // The scrollbar is there, wider than the stage; moving it scrolls the view and stops the following.
  const bar = page.getByTestId('arr-scrollbar')
  await expect(bar).toBeVisible()
  expect(await bar.evaluate((e) => e.scrollWidth > e.clientWidth)).toBe(true)
  // The room of the bar is kept (headless browsers hide scrollbars, so the paint itself is not measured).
  expect((await bar.boundingBox())!.height).toBeGreaterThanOrEqual(12)
  await bar.evaluate((e) => (e.scrollLeft = 400))
  await expect.poll(scroll).toBeGreaterThan(0)
  await expect(arr(page)).toHaveAttribute('data-follow', 'false')
  await page.getByTestId('arr-follow').click()
  await expect(arr(page)).toHaveAttribute('data-follow', 'true')
  await grid.focus()
  await page.keyboard.press('Home')
  await expect.poll(scroll).toBe(0)
  await expect(arr(page)).toHaveAttribute('data-follow', 'false')

  // Play: the view turns the page when the position leaves the window (the next Play follows again).
  await play()
  await expect(arr(page)).toHaveAttribute('data-follow', 'true')
  await expect.poll(scroll, { timeout: 30_000 }).toBeGreaterThan(0)
  const turned = await scroll()
  const ppt = await num('data-px-per-tick')
  const width = (await stage(page).boundingBox())!.width
  const position = Number(await arr(page).getAttribute('data-position'))
  expect(position).toBeGreaterThanOrEqual(turned)
  expect(position).toBeLessThanOrEqual(turned + width / ppt)
  // The bar is drawn: the view followed it.
  await expect.poll(async () => (await grid.getAttribute('data-playhead-x')) ?? '').not.toBe('')

  // A hand scroll (Shift + wheel) stops the following: the view stays where the hand left it while the position runs on.
  const box = (await stage(page).boundingBox())!
  await page.mouse.move(box.x + 200, box.y + 100)
  await page.keyboard.down('Shift')
  await page.mouse.wheel(0, 200)
  await page.keyboard.up('Shift')
  await expect(arr(page)).toHaveAttribute('data-follow', 'false')
  await expect.poll(scroll).toBeGreaterThan(turned)
  const held = await scroll()
  await expect.poll(async () => Number(await arr(page).getAttribute('data-position')), { timeout: 30_000 }).toBeGreaterThan(held + width / ppt)
  expect(await scroll()).toBe(held)
  // The bar is out of the window: the hand scroll let it leave.
  expect((await grid.getAttribute('data-playhead-x')) ?? '').toBe('')

  // Follow brings the position back into the window.
  await page.getByTestId('arr-follow').click()
  await expect(arr(page)).toHaveAttribute('data-follow', 'true')
  await expect.poll(scroll).toBeGreaterThan(held)
  await expect.poll(async () => (await grid.getAttribute('data-playhead-x')) ?? '').not.toBe('')
  // A zoom while following keeps the following, and the browser's clamping of the scrollbar is not taken for a hand.
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  await expect(arr(page)).toHaveAttribute('data-follow', 'true')
  await stop()

  // The cost of a frame stays the grid's: no redraw of the grid while the position moves within a page.
  await play()
  const before = await num('data-draw-count')
  await page.waitForTimeout(800)
  expect((await num('data-draw-count')) - before).toBeLessThan(8)
  expect(Number(await grid.getAttribute('data-body-ms'))).toBeLessThan(process.env.FAULK_E2E_STRICT_TIMING === '1' ? 4 : 50)
  await stop()
  expect(seen.errors).toEqual([])
})

// #395: with Follow on, Stop, Back to start and a seek bring the view to the play position when it is outside the window;
// with Follow off the view does not move.
test('arrangement view: Stop, Back to start and a seek bring the view back to the position while following', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('nav-arrangement').click()
  const grid = stage(page).getByTestId('timegrid')
  const num = async (attribute: string) => Number(await grid.getAttribute(attribute))
  const scroll = () => num('data-scroll-tick')
  const visible = async () => ((await grid.getAttribute('data-playhead-x')) ?? '') !== ''
  const transport = page.getByTestId('transport')
  const play = () => transport.getByRole('button', { name: 'Play', exact: true }).click()
  const stop = () => transport.getByRole('button', { name: 'Stop', exact: true }).click()
  await expect.poll(() => num('data-draw-count')).toBeGreaterThan(0)

  // Play until the view turns the page, then Stop: the position goes back to the start and the view with it.
  await play()
  await expect.poll(scroll, { timeout: 30_000 }).toBeGreaterThan(0)
  await stop()
  await expect(arr(page)).toHaveAttribute('data-follow', 'true')
  await expect.poll(scroll).toBe(0)
  await expect.poll(visible).toBe(true)

  // Back to start while playing, past the first page: the position jumps to 0 and the view follows it.
  await play()
  await expect.poll(scroll, { timeout: 30_000 }).toBeGreaterThan(0)
  await transport.getByTestId('transport-rewind').click()
  await expect.poll(scroll).toBe(0)
  await expect.poll(visible).toBe(true)

  // Follow off (a hand scroll): Stop and Back to start leave the view where it is, the position goes out of the window.
  await expect.poll(scroll, { timeout: 30_000 }).toBeGreaterThan(0)
  const box = (await stage(page).boundingBox())!
  await page.mouse.move(box.x + 200, box.y + 100)
  await page.keyboard.down('Shift')
  await page.mouse.wheel(0, 200)
  await page.keyboard.up('Shift')
  await expect(arr(page)).toHaveAttribute('data-follow', 'false')
  const held = await scroll()
  await stop()
  await transport.getByTestId('transport-rewind').click()
  await page.waitForTimeout(500)
  expect(await scroll()).toBe(held)
  expect(await visible()).toBe(false)

  // A seek far outside the window needs the dispatcher hook of the dev server: with Follow off the view stays,
  // with Follow on it goes to the position.
  const hooked = await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun === 'function')
  if (hooked) {
    const seek = (tick: number) =>
      page.evaluate((t) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun({ type: 'transport.seek', tick: t }), tick)
    await seek(10 * BAR)
    await page.waitForTimeout(500)
    expect(await scroll()).toBe(held)
    expect(await visible()).toBe(false)
    await page.getByTestId('arr-follow').click()
    await expect(arr(page)).toHaveAttribute('data-follow', 'true')
    await expect.poll(visible).toBe(true)
    await seek(0)
    await expect.poll(scroll).toBe(0)
    await expect.poll(visible).toBe(true)
    await seek(10 * BAR)
    await expect.poll(scroll).toBeGreaterThan(0)
    await expect.poll(visible).toBe(true)
  }
  // Follow brings the stopped position back too, and a click on the ruler still moves it.
  await page.getByTestId('arr-follow').click()
  await expect(arr(page)).toHaveAttribute('data-follow', 'true')
  await expect.poll(visible).toBe(true)
  const g = await spot(page)
  await page.mouse.click(g.x(0) + 200, g.ruler)
  await expect.poll(visible).toBe(true)
  expect(seen.errors).toEqual([])
})

// #438: the track headers and the clip canvas are one vertical scroll. Rows that overflow a short window show a
// vertical scrollbar, and every way to scroll (wheel on the canvas, wheel on the headers, the scrollbar, a focus
// in the headers) moves both together: each header row stays level with its lane of the canvas.
test('arrangement view: the headers and the canvas scroll together, with a vertical scrollbar', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1400, height: 560 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  for (let i = 0; i < 6; i++) {
    await page.getByTestId('add-track').click()
    await page.getByRole('menuitem', { name: 'Drums', exact: true }).click()
  }
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toBeVisible()
  await expect.poll(async () => Number(await stage(page).getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)

  const heads = page.getByTestId('arr-heads')
  const rows = heads.locator('.row')
  const items = heads.locator('.slide > *')
  const vbar = page.getByTestId('arr-vscrollbar')
  await expect(vbar).toBeVisible()
  const scrollY = async () => Number(await stage(page).getAttribute('data-scroll-y'))
  const max = async () => Number(await vbar.getAttribute('aria-valuemax'))
  expect(await max()).toBeGreaterThan(0)

  /** Every header row sits where the canvas draws its lane (the rows start under the ruler and move by the scroll). */
  async function aligned() {
    const top = (await stage(page).boundingBox())!.y
    const y = await scrollY()
    expect(await heads.evaluate((e) => e.scrollTop)).toBe(0)
    const count = await items.count()
    expect(count).toBeGreaterThanOrEqual(7)
    // Every row of the column, the master and its lanes included, follows the one before it from under the ruler.
    let offset = 0
    for (let i = 0; i < count; i++) {
      const box = (await items.nth(i).boundingBox())!
      expect(Math.abs(box.y - (top + RULER + offset - y))).toBeLessThanOrEqual(1)
      // The six tracks are as tall as the canvas's lanes.
      if (i < 6) expect(Math.abs(box.height - LANE)).toBeLessThanOrEqual(1)
      offset += box.height
    }
    // The scrollbar says the same thing.
    await expect.poll(() => vbar.evaluate((e) => e.scrollTop)).toBeCloseTo(y, 0)
  }
  await aligned()

  // The wheel on the headers scrolls like the wheel on the canvas.
  await heads.hover()
  await page.mouse.wheel(0, 100)
  await expect.poll(scrollY).toBe(100)
  await aligned()
  await stage(page).hover({ position: { x: 300, y: 150 } })
  await page.mouse.wheel(0, 40)
  await expect.poll(scrollY).toBe(140)
  await aligned()

  // The scrollbar drives both.
  await vbar.evaluate((e) => (e.scrollTop = 30))
  await expect.poll(scrollY).toBe(30)
  await aligned()

  // A scroll of the headers column alone (what a focus or scrollIntoView does) becomes the view's scroll.
  await heads.evaluate((e) => (e.scrollTop = 20))
  await expect.poll(scrollY).toBe(50)
  await aligned()

  // Focus on a header button out of the window brings the canvas with it.
  await vbar.evaluate((e) => (e.scrollTop = 0))
  await expect.poll(scrollY).toBe(0)
  await rows.nth(6).getByRole('button').first().focus()
  await expect.poll(scrollY).toBeGreaterThan(0)
  await aligned()

  // To the end: the last row (the master) is reachable and still level.
  await heads.hover()
  await page.mouse.wheel(0, 5000)
  await expect.poll(scrollY).toBe(await max())
  await aligned()

  // Focus on a header above the window (Shift+Tab from the master) brings the canvas back up.
  await heads.getByTestId('arr-master-row').getByRole('button').focus()
  await expect.poll(scrollY).toBe(await max())
  await rows.nth(0).getByRole('button').first().focus()
  await expect.poll(scrollY).toBe(0)
  await aligned()
  await heads.getByTestId('arr-master-row').getByRole('button').focus()
  await expect.poll(scrollY).toBe(await max())
  await page.keyboard.press('Shift+Tab')
  await aligned()

  // The master's lanes unfolded in this short window: they are reachable and stay level with the canvas.
  const toggle = heads.getByTestId('arr-master-row').getByTestId('arr-automation-toggle')
  await toggle.click()
  await heads.getByTestId('arr-add-lane').last().selectOption({ label: 'Volume' })
  await expect(page.getByTestId('arr-lane')).toHaveCount(1)
  await expect.poll(async () => (await max()) > 0).toBe(true)
  await heads.hover()
  await page.mouse.wheel(0, 5000)
  await expect.poll(scrollY).toBe(await max())
  await aligned()
  await expect(page.getByTestId('arr-lane')).toBeInViewport()
  expect(seen.errors).toEqual([])
})
