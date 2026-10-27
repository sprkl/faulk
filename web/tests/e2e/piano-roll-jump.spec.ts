import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #369: the grid does not move when the first note appears (the toolbar keeps its height), and the
// velocity lane picks the right note when two share a step: the selected one, else the highest.

const ROW = 16
const RULER = 28

interface Note {
  start: number
  pitch: number
  length: number
  velocity: number
}

const roll = (page: Page) => page.getByTestId('piano-roll')
const lane = (page: Page) => page.getByTestId('roll-lane')
const notes = async (page: Page): Promise<Note[]> => JSON.parse((await roll(page).getAttribute('data-notes')) ?? '[]')
const velocityOf = async (page: Page, pitch: number) => (await notes(page)).find((n) => n.pitch === pitch)!.velocity

async function open(page: Page, width = 1280) {
  const seen = watch(page)
  await page.setViewportSize({ width, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  return seen
}

async function spot(page: Page) {
  const box = (await page.getByTestId('roll-stage').boundingBox())!
  const ppt = Number(await page.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await page.getByTestId('roll-stage').getAttribute('data-scroll-y'))
  return { x: (tick: number) => box.x + tick * ppt, y: (pitch: number) => box.y + RULER + (127 - pitch) * ROW - scrollY + ROW / 2 }
}

async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps: 6 })
  await page.mouse.up()
}

// 1280 px is the width of the issue; 1500 px is one where the count used to wrap to a new line on the first note.
for (const width of [1280, 1440, 1500]) {
  test(`piano roll: the first note does not move the toolbar or the grid at ${width} px`, async ({ page }) => {
    await firstNote(page, width)
  })
}

async function firstNote(page: Page, width: number) {
  const seen = await open(page, width)
  const grid = page.getByTestId('roll-stage')
  const toolbar = roll(page).locator('.toolbar')
  const before = { grid: (await grid.boundingBox())!, toolbar: (await toolbar.boundingBox())! }
  const g = await spot(page)
  await page.mouse.click(g.x(960), g.y(60))
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  await expect(page.getByTestId('roll-count')).toContainText('1')
  const after = { grid: (await grid.boundingBox())!, toolbar: (await toolbar.boundingBox())! }
  expect(after.toolbar.height).toBe(before.toolbar.height)
  expect(after.grid.y).toBe(before.grid.y)
  expect(after.grid.height).toBe(before.grid.height)
  expect(seen.errors).toEqual([])
}

test('piano roll: two notes on one step, the lane changes the selected note, else the highest, as one undo step', async ({ page }) => {
  const seen = await open(page)
  const g = await spot(page)
  await page.mouse.click(g.x(960), g.y(60))
  await page.mouse.click(g.x(960), g.y(64))
  await expect(roll(page)).toHaveAttribute('data-note-count', '2')
  // Nothing selected: Escape on the grid clears the selection.
  await page.getByTestId('timegrid').focus()
  await page.keyboard.press('Escape')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')

  const laneBox = (await lane(page).boundingBox())!
  const barX = g.x(960) + 1
  const from = { x: barX, y: laneBox.y + 13 }
  // The bar that a drag would change is lit while the pointer hovers it: the highest note.
  await page.mouse.move(from.x, from.y)
  await expect(lane(page)).toHaveAttribute('data-hover', /^960:64:/)

  await drag(page, from, { x: barX, y: laneBox.y + 40 })
  await expect.poll(() => velocityOf(page, 64)).toBeLessThan(100)
  expect(await velocityOf(page, 60)).toBe(100)
  const lowered = await velocityOf(page, 64)

  // Select the lower note: the same drag now changes it, and not the other one.
  await page.mouse.click(g.x(965), g.y(60))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')
  await page.mouse.move(from.x, from.y)
  await expect(lane(page)).toHaveAttribute('data-hover', /^960:60:/)
  await drag(page, from, { x: barX, y: laneBox.y + 40 })
  await expect.poll(() => velocityOf(page, 60)).toBeLessThan(100)
  expect(await velocityOf(page, 64)).toBe(lowered)
  await expect(roll(page)).toHaveAttribute('data-selected-count', '1')

  // One undo step for the whole drag.
  await page.keyboard.press('Control+z')
  await expect.poll(() => velocityOf(page, 60)).toBe(100)
  expect(await velocityOf(page, 64)).toBe(lowered)

  // Nothing selected: a double click on the bar resets the highest note of the step, as one undo step.
  await page.getByTestId('timegrid').focus()
  await page.keyboard.press('Escape')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')
  await lane(page).dblclick({ position: { x: barX - laneBox.x, y: 20 } })
  await expect.poll(() => velocityOf(page, 64)).toBe(100)
  expect(await velocityOf(page, 60)).toBe(100)
  await page.keyboard.press('Control+z')
  await expect.poll(() => velocityOf(page, 64)).toBe(lowered)
  expect(seen.errors).toEqual([])
})
