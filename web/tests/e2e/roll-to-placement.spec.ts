import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #944: from the piano roll, Alt+P goes to the arrangement with the open clip's placement selected, in sight and holding the keyboard
// focus, so that Alt+Shift+Left (cut the end) acts on it with no other action. The command palette lists the same gesture. A clip with no
// placement says so and nothing moves. Real worklet, engine and project core, read back from the page; the console stays clean.

test.use({ storageState: { cookies: [], origins: [] } })

const BAR = 3840
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
type Block = { id: string; row: number; start: number; length: number; clip: string }
const blocks = async (page: Page): Promise<Block[]> => JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')
const picked = async (page: Page): Promise<string[]> => JSON.parse((await page.getByTestId('arrangement').getAttribute('data-selection')) ?? '[]')

async function start(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function', 'the dev server only')
}

/** Nothing focused, as after a click on the page: the keys go to the page, which the roll listens to when it is open. */
const unfocus = (page: Page) => page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())

test('Alt+P in the piano roll selects and focuses the open clip’s placement, and Alt+Shift+Left then cuts its end', async ({ page }) => {
  const { errors } = watch(page)
  test.setTimeout(120_000)
  await start(page)
  // The clip is placed three times; the roll opens on the first placement.
  await page.getByTestId('track-header').first().click()
  await run(page, { type: 'clip.create', track: await firstTrack(page), at: { at: 'arrangement', start: 0 }, length: BAR })
  const clip = (await blocks(page))[0]!.clip
  await run(page, { type: 'clip.notes.add', clip, notes: [{ start: 0, pitch: 60, length: BAR / 4, velocity: 100, cents: 0, chance: 100, repeat: 1 }] })
  await run(page, { type: 'arrangement.place', clip, start: 3 * BAR, copy: false })
  await run(page, { type: 'arrangement.place', clip, start: 6 * BAR, copy: false })
  await expect.poll(async () => (await blocks(page)).length).toBe(3)
  const all = (await blocks(page)).sort((a, b) => a.start - b.start)

  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  await unfocus(page)
  await page.keyboard.press('Alt+p')

  const first = all[0]!
  await expect.poll(() => picked(page)).toEqual([first.id])
  await expect(page.locator(`[data-testid=arr-item][data-block="${first.id}"]`)).toBeFocused()

  // The key of the arrangement acts on that placement, and on it alone.
  await page.keyboard.press('Alt+Shift+ArrowLeft')
  await expect.poll(async () => (await blocks(page)).find((b) => b.id === first.id)!.length).toBeLessThan(BAR)
  const after = await blocks(page)
  expect(after.find((b) => b.id === all[1]!.id)!.length).toBe(BAR)
  expect(after.find((b) => b.id === all[2]!.id)!.length).toBe(BAR)
  expect(errors).toEqual([])
})

test('without an origin the placement nearest to the playhead is shown, and the one the clip was opened from wins over it', async ({ page }) => {
  const { errors } = watch(page)
  test.setTimeout(120_000)
  await start(page)
  await page.getByTestId('track-header').first().click()
  await run(page, { type: 'clip.create', track: await firstTrack(page), at: { at: 'arrangement', start: 0 }, length: BAR })
  const clip = (await blocks(page))[0]!.clip
  await run(page, { type: 'arrangement.place', clip, start: 3 * BAR, copy: false })
  await run(page, { type: 'arrangement.place', clip, start: 6 * BAR, copy: false })
  await expect.poll(async () => (await blocks(page)).length).toBe(3)
  const all = (await blocks(page)).sort((a, b) => a.start - b.start)
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('piano-roll')).toBeVisible()

  // The playhead in the third placement: it is the one shown (the roll was not opened from the timeline).
  await run(page, { type: 'transport.seek', tick: 6 * BAR + BAR / 2 })
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-position', String(6 * BAR + BAR / 2))
  await unfocus(page)
  await page.keyboard.press('Alt+p')
  await expect.poll(() => picked(page)).toEqual([all[2]!.id])
  await expect(page.locator(`[data-testid=arr-item][data-block="${all[2]!.id}"]`)).toBeFocused()

  // Opened from the second one (Enter on its cell), the roll goes back to it whatever the playhead says.
  const second = page.locator(`[data-testid=arr-item][data-block="${all[1]!.id}"]`)
  await second.focus()
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  await run(page, { type: 'transport.seek', tick: 6 * BAR + BAR / 2 })
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-position', String(6 * BAR + BAR / 2))
  await unfocus(page)
  await page.keyboard.press('Alt+p')
  await expect.poll(() => picked(page)).toEqual([all[1]!.id])
  await expect(second).toBeFocused()
  expect(errors).toEqual([])
})

test('the palette lists "Go to the clip’s placement" and does the same', async ({ page }) => {
  const { errors } = watch(page)
  test.setTimeout(120_000)
  await start(page)
  await page.getByTestId('track-header').first().click()
  await run(page, { type: 'clip.create', track: await firstTrack(page), at: { at: 'arrangement', start: 2 * BAR }, length: BAR })
  const placed = (await blocks(page))[0]!
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  await unfocus(page)
  await page.keyboard.press('Control+k')
  await page.getByTestId('palette-search').fill('placement')
  await page.getByRole('option', { name: /Go to the clip’s placement/ }).click()
  await expect.poll(() => picked(page)).toEqual([placed.id])
  await expect(page.locator(`[data-testid=arr-item][data-block="${placed.id}"]`)).toBeFocused()
  expect(errors).toEqual([])
})

test('a clip with no placement says so and nothing moves', async ({ page }) => {
  const { errors } = watch(page)
  test.setTimeout(120_000)
  await start(page)
  await page.getByTestId('track-header').first().click()
  await run(page, { type: 'scene.create' })
  const project = await page.evaluate(
    () => (window as unknown as { __faulkView(): { project: { tracks: Record<string, unknown>; scenes: Record<string, unknown> } } }).__faulkView().project,
  )
  await run(page, { type: 'clip.create', track: Object.keys(project.tracks)[0], at: { at: 'slot', scene: Object.keys(project.scenes)[0] } })
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  await unfocus(page)
  await page.keyboard.press('Alt+p')
  await expect(page.getByTestId('roll-said')).toHaveText(/not placed in the arrangement/)
  await expect(page.getByTestId('piano-roll')).toBeVisible()
  expect(await picked(page)).toEqual([])
  expect(errors).toEqual([])
})

async function firstTrack(page: Page): Promise<string> {
  return page.evaluate(
    () => Object.keys((window as unknown as { __faulkView(): { project: { tracks: Record<string, unknown> } } }).__faulkView().project.tracks)[0]!,
  )
}
