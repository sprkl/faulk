import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #360: Stop brings the playhead back to where playback started (the marker: the last position laid on the ruler,
// 1.1.000 at first), and a second Stop to 1.1.000, so the position shown is always where Play restarts. Space stays
// a pause. Real worklet and engine; the position is read on the transport bar, on the arrangement (`data-position`)
// and on the engine's report (`feedback`), which the piano roll's bar reads too.

const BAR = 3840
const feedback = (page: Page) => page.getByTestId('feedback')
const tick = async (page: Page) => Number(await feedback(page).getAttribute('data-tick'))
const state = (page: Page, s: 'playing' | 'stopped') => expect(feedback(page)).toHaveAttribute('data-transport', s)
const button = (page: Page, name: string) => page.getByTestId('transport').getByRole('button', { name, exact: true })
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
const arrangementPosition = async (page: Page) => Number(await page.getByTestId('arrangement').getAttribute('data-position'))

async function start(page: Page) {
  await page.setViewportSize({ width: 1600, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('nav-arrangement').click()
}

/** Lays the marker with a real click on the arrangement ruler, near the start of bar 2, and returns its tick. */
async function layMarker(page: Page): Promise<number> {
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.click(box.x + BAR * ppt, box.y + 14)
  await expect.poll(() => arrangementPosition(page)).toBeGreaterThan(BAR - 400)
  const marker = await arrangementPosition(page)
  expect(marker).toBeLessThan(BAR + 400)
  await expect.poll(() => tick(page)).toBe(marker)
  return marker
}

test('Stop returns to the marker, a second Stop to 1.1.000, and Play restarts from what is shown', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const marker = await layMarker(page)
  const position = page.getByTestId('transport-position')
  const shown = await position.textContent()

  await button(page, 'Play').click()
  await state(page, 'playing')
  await expect.poll(() => tick(page)).toBeGreaterThan(marker + 1500)
  await button(page, 'Stop').click()
  await state(page, 'stopped')
  // The three displays are on the marker, and they stay there.
  await expect.poll(() => tick(page)).toBe(marker)
  await page.waitForTimeout(300)
  expect(await tick(page)).toBe(marker)
  expect(await arrangementPosition(page)).toBe(marker)
  await expect(position).toHaveText(shown!)

  // Play starts from what is shown.
  await button(page, 'Play').click()
  await state(page, 'playing')
  await page.waitForTimeout(300)
  const after = await tick(page)
  expect(after).toBeGreaterThanOrEqual(marker)
  expect(after).toBeLessThan(marker + 3000)

  // Stop again: back on the marker; Stop once more: 1.1.000, and Play follows.
  await button(page, 'Stop').click()
  await state(page, 'stopped')
  await expect.poll(() => tick(page)).toBe(marker)
  await button(page, 'Stop').click()
  await expect.poll(() => tick(page)).toBe(0)
  await expect(position).toHaveText('1.1.000')
  expect(await arrangementPosition(page)).toBe(0)
  await button(page, 'Play').click()
  await state(page, 'playing')
  await page.waitForTimeout(300)
  expect(await tick(page)).toBeLessThan(3000)
  await button(page, 'Stop').click()
  await expect.poll(() => tick(page)).toBe(0)
  expect(errors).toEqual([])
})

test('Space stays a pause that keeps the marker: pause, resume, Stop goes back to the marker', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const marker = await layMarker(page)
  await page.getByTestId('view').click({ position: { x: 5, y: 5 } })
  await page.keyboard.press('Space')
  await state(page, 'playing')
  await expect.poll(() => tick(page)).toBeGreaterThan(marker + 2500)
  await page.keyboard.press('Space')
  await state(page, 'stopped')
  await page.waitForTimeout(400)
  const paused = await tick(page)
  expect(paused).toBeGreaterThan(marker + 2500)
  await page.waitForTimeout(300)
  expect(await tick(page)).toBe(paused)
  expect(await arrangementPosition(page)).toBe(paused)
  // Resume: from the pause, not from the marker.
  await page.keyboard.press('Space')
  await state(page, 'playing')
  await page.waitForTimeout(300)
  expect(await tick(page)).toBeGreaterThanOrEqual(paused)
  // Stop returns to the marker, not to the pause point.
  await button(page, 'Stop').click()
  await state(page, 'stopped')
  await expect.poll(() => tick(page)).toBe(marker)
  await button(page, 'Stop').click()
  await expect.poll(() => tick(page)).toBe(0)
  expect(errors).toEqual([])
})

test('pause then Stop goes back to the marker, and the next Stop to 1.1.000', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const marker = await layMarker(page)
  await page.getByTestId('view').click({ position: { x: 5, y: 5 } })
  await page.keyboard.press('Space')
  await state(page, 'playing')
  await expect.poll(() => tick(page)).toBeGreaterThan(marker + 2500)
  await page.keyboard.press('Space')
  await state(page, 'stopped')
  await page.waitForTimeout(400)
  expect(await tick(page)).toBeGreaterThan(marker + 2500)
  await button(page, 'Stop').click()
  await expect.poll(() => tick(page)).toBe(marker)
  await expect.poll(() => arrangementPosition(page)).toBe(marker)
  await button(page, 'Stop').click()
  await expect.poll(() => tick(page)).toBe(0)
  expect(errors).toEqual([])
})

test('a double-click on Stop still stops, cuts all sound and ends on 1.1.000', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await layMarker(page)
  await button(page, 'Play').click()
  await state(page, 'playing')
  await expect.poll(() => tick(page)).toBeGreaterThan(BAR + 1500)
  await button(page, 'Stop').dblclick()
  await state(page, 'stopped')
  // click, click, dblclick: marker, then the start of the project.
  await expect.poll(() => tick(page)).toBe(0)
  await expect(page.getByTestId('transport-position')).toHaveText('1.1.000')
  expect(errors).toEqual([])
})

test('grid: a scene launched while stopped plays from the marker and Stop returns to it', async ({ page }) => {
  test.setTimeout(120_000)
  const { errors } = watch(page)
  await start(page)
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
    'the dispatcher hook exists in the dev server only',
  )
  const marker = 2 * BAR
  await run(page, { type: 'transport.seek', tick: marker })
  await expect.poll(() => tick(page)).toBe(marker)
  // A scene with a held-note clip on the first track.
  await run(page, { type: 'scene.create' })
  const project = await page.evaluate(
    () =>
      (window as unknown as { __faulkView(): { project: { tracks: Record<string, { kind: string }>; scenes: Record<string, unknown> } } }).__faulkView()
        .project,
  )
  const track = Object.entries(project.tracks).find(([, t]) => t.kind === 'midi')![0]
  const scene = Object.keys(project.scenes)[0]!
  await run(page, { type: 'clip.create', track, at: { at: 'slot', scene } })
  const clip = await page.evaluate(
    (id) =>
      Object.keys(
        (window as unknown as { __faulkView(): { project: { tracks: Record<string, { clips: Record<string, unknown> }> } } }).__faulkView().project.tracks[id]!
          .clips,
      )[0]!,
    track,
  )
  await run(page, { type: 'clip.notes.add', clip, notes: [{ start: 0, pitch: 60, length: 3800, velocity: 100 }] })
  await page.getByTestId('nav-grid').click()
  // The first scene of the default project: launch it from its row.
  await page.getByTestId('launch-scene').first().click()
  await state(page, 'playing')
  await expect.poll(() => tick(page)).toBeGreaterThan(marker)
  await button(page, 'Stop').click()
  await state(page, 'stopped')
  await expect.poll(() => tick(page)).toBe(marker)
  await page.waitForTimeout(300)
  expect(await tick(page)).toBe(marker)
  await button(page, 'Stop').click()
  await expect.poll(() => tick(page)).toBe(0)
  expect(errors).toEqual([])
})

test('a new project resets the engine: the head shown is where Play starts', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
    'the dispatcher hook exists in the dev server only',
  )
  await run(page, { type: 'transport.seek', tick: 2 * BAR })
  await expect.poll(() => tick(page)).toBe(2 * BAR)
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-new').click()
  const discard = page.getByTestId('confirm-discard')
  if (await discard.isVisible().catch(() => false)) await discard.click()
  await expect.poll(() => tick(page)).toBe(0)
  await expect(page.getByTestId('transport-position')).toHaveText('1.1.000')
  expect(errors).toEqual([])
})
