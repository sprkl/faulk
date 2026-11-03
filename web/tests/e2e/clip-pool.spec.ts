import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #473: one clip pool per track. A clip no longer dies with its last placement (the PO's case of 2026-11-02): it stays
// in the Clips panel, from where it goes back to the arrangement or into the grid, by a drag or a button. A stretched
// clip repeats its content, and a linked clip is marked.

test.describe.configure({ timeout: 120_000 })

const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
type View = {
  project: {
    scenes: Record<string, { name: string }>
    tracks: Record<
      string,
      { name: string; clips: Record<string, { name: string; slot: string | null }>; placements: Record<string, { clip: string; start: number }> }
    >
  }
}
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())

async function start(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
}

test('a clip outlives its last placement, comes back from the pool, and repeats when stretched', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the command hook exists in the dev server only')
  const track = Object.keys((await view(page)).project.tracks)[0]!

  // The PO's steps: a clip with notes in the arrangement, all its placements deleted.
  await run(page, { type: 'clip.create', track, at: { at: 'arrangement', start: 0 } })
  await expect.poll(async () => Object.keys((await view(page)).project.tracks[track]!.clips).length).toBe(1)
  const clip = Object.keys((await view(page)).project.tracks[track]!.clips)[0]!
  await run(page, { type: 'clip.notes.add', clip, notes: [{ start: 0, pitch: 60, length: 480, velocity: 100 }] })
  const placements = Object.keys((await view(page)).project.tracks[track]!.placements)
  await run(page, { type: 'arrangement.delete', placements })
  await expect.poll(async () => Object.keys((await view(page)).project.tracks[track]!.placements).length).toBe(0)
  // The clip is still there, in the pool.
  expect(Object.keys((await view(page)).project.tracks[track]!.clips)).toEqual([clip])
  await page.getByTestId('panel-clips').click()
  const row = page.locator(`[data-testid=pool-clip][data-clip="${clip}"]`)
  await expect(row).toContainText((await view(page)).project.tracks[track]!.clips[clip]!.name)
  await expect(row).toContainText('Not placed')
  await expect(row).toHaveAttribute('data-uses', '0')

  // Dragged back onto the lane, at bar 2: the clip itself (it was used nowhere).
  await page.getByTestId('nav-arrangement').click()
  const stage = page.getByTestId('arr-stage')
  const grid = stage.getByTestId('timegrid')
  await expect.poll(async () => Number(await grid.getAttribute('data-draw-count'))).toBeGreaterThan(0)
  const ppt = Number(await grid.getAttribute('data-px-per-tick'))
  const header = page.locator(`[data-testid=track-header][data-track="${track}"]`)
  const headerBox = (await header.boundingBox())!
  const stageBox = (await stage.boundingBox())!
  await row.getByTestId('pool-handle').dragTo(stage, { targetPosition: { x: 3840 * ppt + 10, y: headerBox.y - stageBox.y + headerBox.height / 2 } })
  await expect.poll(async () => Object.values((await view(page)).project.tracks[track]!.placements).map((p) => p.clip)).toEqual([clip])
  // At the snapped tick under the pointer, in bar 2.
  const at = Object.values((await view(page)).project.tracks[track]!.placements)[0]!.start
  expect(at).toBeGreaterThanOrEqual(3840)
  expect(at).toBeLessThan(3840 * 2)
  await expect(row).toContainText('1 in the arrangement')

  // Stretched to four bars by its end: the content repeats, said and drawn.
  const placement = Object.keys((await view(page)).project.tracks[track]!.placements)[0]!
  await run(page, { type: 'arrangement.resize', placement, edge: 'end', to: 3840 * 5 })
  await expect(page.locator(`[data-testid=arr-item][data-block="${placement}"]`)).toHaveAccessibleName(/plays its content 4 times/)

  // Placed again with Alt from the button: a linked clip, marked in the panel and on the block.
  await page.getByTestId('panel-clips').click()
  await page.keyboard.down('Alt')
  await row.getByTestId('pool-place').click()
  await page.keyboard.up('Alt')
  await expect(row).toHaveAttribute('data-uses', '2')
  await expect(row.getByTestId('link-mark')).toBeVisible()
  await expect(page.locator(`[data-testid=arr-item][data-block="${placement}"]`)).toHaveAccessibleName(/linked/)

  // Without Alt, a used clip is copied: a new clip.
  await row.getByTestId('pool-place').click()
  await expect.poll(async () => Object.keys((await view(page)).project.tracks[track]!.clips).length).toBe(2)

  // Into the grid: the copy goes in a free cell; a cell of the track takes a drag from the pool.
  await run(page, { type: 'scene.create' })
  await run(page, { type: 'scene.create' })
  await page.getByTestId('nav-grid').click()
  await expect(page.getByTestId('grid')).toBeVisible()
  const cell = page.locator('[data-testid=cell][data-ti="0"][data-si="1"]')
  await row.getByTestId('pool-handle').dragTo(cell)
  await expect.poll(async () => Object.values((await view(page)).project.tracks[track]!.clips).filter((c) => c.slot !== null).length).toBe(1)

  // Deleted from the pool: undoable.
  const before = Object.keys((await view(page)).project.tracks[track]!.clips).length
  await row.getByTestId('pool-delete').click()
  await expect.poll(async () => Object.keys((await view(page)).project.tracks[track]!.clips).length).toBe(before - 1)
  expect(errors).toEqual([])
})
