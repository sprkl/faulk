import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #293: the play position in the piano roll, in a real browser with the real worklet and engine. A clip of the
// arrangement and a clip launched from the grid each get a vertical bar that advances with the transport (the grid
// clip's wrapping on its loop), stays where it was when the transport stops, and costs one clear and one line a frame
// on its own canvas: the grid and the notes are not redrawn while it moves. The numbers are printed.

const ROW = 16
const RULER = 28

const startAudio = (page: Page) => page.getByTestId('engine-state').getByRole('button', { name: /Start audio|Démarrer/ }).click()
const transport = (page: Page, name: string) => page.getByTestId('transport').getByRole('button', { name, exact: true })
const timegrid = (page: Page) => page.getByTestId('timegrid')
const cell = (page: Page, scene: number) => page.getByTestId('scene-row').nth(scene).getByTestId('cell').first()
const barX = async (page: Page) => (await timegrid(page).getAttribute('data-playhead-x')) ?? ''

interface Sampled {
  /** The bar's x at each animation frame where it was drawn (CSS px). */
  xs: number[]
  /** The cost of each pass of the overlay, in ms. */
  costs: number[]
  /** Passes of the overlay and redraws of the grid and notes during the sampling. */
  passes: number
  gridDraws: number
  frames: number
}

/** Samples the bar on every animation frame for `ms`. */
function sample(page: Page, ms: number): Promise<Sampled> {
  return page.evaluate(
    (duration) =>
      new Promise<Sampled>((done) => {
        const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
        const out: Sampled = { xs: [], costs: [], passes: 0, gridDraws: 0, frames: 0 }
        const firstPass = Number(root.dataset.playheadCount)
        const firstDraw = Number(root.dataset.drawCount)
        let lastPass = firstPass
        const begin = performance.now()
        const frame = (now: number) => {
          out.frames++
          const pass = Number(root.dataset.playheadCount)
          if (pass !== lastPass) out.costs.push(Number(root.dataset.playheadMs))
          lastPass = pass
          if (root.dataset.playheadX) out.xs.push(Number(root.dataset.playheadX))
          if (now - begin < duration) requestAnimationFrame(frame)
          else {
            out.passes = pass - firstPass
            out.gridDraws = Number(root.dataset.drawCount) - firstDraw
            done(out)
          }
        }
        requestAnimationFrame(frame)
      }),
    ms,
  )
}

const median = (values: number[]) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)] ?? 0
const report = (label: string, s: Sampled) =>
  console.log(
    `playhead ${label}: ${s.frames} frames, ${s.passes} overlay passes (median ${median(s.costs).toFixed(3)} ms, max ${Math.max(0, ...s.costs).toFixed(3)} ms), ` +
      `${s.gridDraws} grid redraws, x ${s.xs[0]} -> ${s.xs[s.xs.length - 1]}`,
  )

/** The cost budget: a clear and a line. A shared runner is noisy, so the ceiling only catches a gross regression. */
function expectCheap(s: Sampled) {
  expect(s.passes).toBeGreaterThan(10)
  expect(Math.max(...s.costs)).toBeLessThan(8)
  expect(median(s.costs)).toBeLessThan(1)
  // The bar moves, the grid and the notes are not redrawn for it.
  expect(s.gridDraws).toBeLessThanOrEqual(2)
}

test('piano roll: the bar of an arrangement clip advances with the transport and stays when it stops', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await timegrid(page).getAttribute('data-draw-count'))).toBeGreaterThan(0)
  // A note, so that the body has something to draw.
  const box = (await page.getByTestId('roll-stage').boundingBox())!
  const ppt = Number(await timegrid(page).getAttribute('data-px-per-tick'))
  const scrollY = Number(await page.getByTestId('roll-stage').getAttribute('data-scroll-y'))
  await page.mouse.click(box.x + 20 * ppt, box.y + RULER + (127 - 60) * ROW - scrollY + ROW / 2)
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '1')

  // Stopped at the start, the bar sits at the clip's first tick (the start point).
  await expect.poll(() => barX(page)).not.toBe('')
  const start = Number(await barX(page))
  expect(start).toBeLessThan(2)

  await transport(page, 'Play').click()
  // The clip is one bar, about two seconds at the default tempo: the bar is sampled over its first second and a half.
  const playing = await sample(page, 1500)
  report('arrangement', playing)
  expect(playing.xs.length).toBeGreaterThan(20)
  const distinct = new Set(playing.xs)
  expect(distinct.size).toBeGreaterThan(20)
  // Forward only (no wrap inside one bar), by at least a few hundred pixels' worth of ticks.
  expect(playing.xs.every((x, i) => i === 0 || x >= playing.xs[i - 1])).toBe(true)
  expect(playing.xs[playing.xs.length - 1] - playing.xs[0]).toBeGreaterThan(100)
  expectCheap(playing)

  await transport(page, 'Stop').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'stopped')
  // Stop brings the bar back to the cue (#360); once it is there it stays: the same place, drawn, with no flicker (a pass that leaves it blank).
  await page.waitForTimeout(300)
  const stopped = await barX(page)
  expect(stopped).not.toBe('')
  const after = await sample(page, 600)
  expect(new Set(after.xs).size).toBe(1)
  expect(after.xs.length).toBe(after.frames)
  expect(await barX(page)).toBe(stopped)
  // The latched bar follows the view: a zoom moves it with the grid, and it is not blanked. Stop left it at the start,
  // where a zoom moves nothing: a seek (the dev server's dispatcher hook) puts it inside the clip first.
  const hooked = await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun === 'function')
  if (hooked) {
    await page.evaluate(() => (window as unknown as { __faulkRun(c: object): void }).__faulkRun({ type: 'transport.seek', tick: 700 }))
    await expect.poll(() => barX(page)).not.toBe(stopped)
  }
  const placed = hooked ? await barX(page) : stopped
  const ppt0 = Number(await timegrid(page).getAttribute('data-px-per-tick'))
  await timegrid(page).focus()
  await page.keyboard.press('+')
  await expect.poll(async () => Number(await timegrid(page).getAttribute('data-px-per-tick'))).toBeGreaterThan(ppt0)
  await expect.poll(() => barX(page)).not.toBe(placed)
  expect(await barX(page)).not.toBe('')
  expect(seen.errors).toEqual([])
})

test('piano roll: the bar of a clip launched from the grid advances, wraps on the loop and stays when the transport stops', async ({ page }) => {
  test.setTimeout(180_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('nav-grid').click()
  await expect(page.getByTestId('grid')).toBeVisible()
  await page.getByTestId('add-scene').click()
  await expect(page.getByTestId('scene-row')).toHaveCount(1)
  // A clip in the cell, by a double-click; the roll opens on it.
  await cell(page, 0).dblclick()
  await expect(cell(page, 0)).toHaveAttribute('data-clip', /.+/)
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  await expect.poll(async () => Number(await timegrid(page).getAttribute('data-draw-count'))).toBeGreaterThan(0)
  expect(await barX(page)).toBe('')

  // Launch it: the transport starts, the clip plays from its boundary and the bar follows it.
  await cell(page, 0).click()
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'playing', { timeout: 10_000 })
  // A clip made by the grid loops (one bar, about two seconds): five seconds show it advance and wrap.
  const playing = await sample(page, 5000)
  report('grid clip', playing)
  expect(new Set(playing.xs).size).toBeGreaterThan(40)
  const rises = playing.xs.filter((x, i) => i > 0 && x > playing.xs[i - 1]).length
  const wraps = playing.xs.filter((x, i) => i > 0 && x < playing.xs[i - 1] - 100)
  expect(rises).toBeGreaterThan(40)
  // At least one wrap back to the start of the loop.
  expect(wraps.length).toBeGreaterThanOrEqual(1)
  expectCheap(playing)

  await transport(page, 'Stop').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'stopped')
  await expect.poll(() => barX(page)).not.toBe('')
  await page.waitForTimeout(300)
  const stopped = await barX(page)
  const after = await sample(page, 600)
  expect(new Set(after.xs).size).toBe(1)
  expect(after.xs.length).toBe(after.frames)
  expect(await barX(page)).toBe(stopped)
  expect(seen.errors).toEqual([])
})
