import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #316: a clip has one name, stored in the project at its creation, shown the same in the grid, the
// timeline and the piano roll, and it does not change when other clips are made elsewhere. The scenario
// is the one of the issue, by gestures: a clip in the grid, two in the arrangement, back to the grid.

const RULER = 28
const LANE = 64
const BAR = 3840

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const stage = (page: Page) => page.getByTestId('arr-stage')
const names = (page: Page) =>
  page.evaluate(() => {
    const view = (window as unknown as { __faulkView(): { project: { tracks: Record<string, { clips: Record<string, { name: string }> }> } } }).__faulkView()
    return Object.values(view.project.tracks)
      .flatMap((t) => Object.values(t.clips).map((c) => c.name))
      .sort()
  })

test('clip names: the same in the grid, the timeline and the piano roll, stable, renamed from the roll', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1800, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkView?: unknown }).__faulkView)) !== 'function',
    'the view hook exists in the dev server only',
  )

  // 1. A clip in the cell of the first scene: "Clip 1".
  await page.getByTestId('nav-grid').click()
  await page.getByTestId('add-scene').click()
  const cell = page.locator('[data-testid=cell][data-ti="0"][data-si="0"]')
  await cell.dblclick()
  await expect(cell).toHaveAttribute('data-clip', /.+/)
  await expect(cell).toContainText('Clip 1')

  // 2. Two clips in the arrangement, bars 1 and 2.
  await page.getByTestId('nav-arrangement').click()
  await expect.poll(async () => Number(await stage(page).getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const y = box.y + RULER + LANE / 2
  await page.mouse.dblclick(box.x + 100 * ppt, y)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '1')
  await page.mouse.dblclick(box.x + (BAR + 100) * ppt, y)
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '2')
  expect(await names(page)).toEqual(['Clip 1', 'Clip 2', 'Clip 3'])

  // 3. Back to the grid: the first clip is still "Clip 1".
  await page.getByTestId('nav-grid').click()
  await expect(cell).toContainText('Clip 1')

  // 4. The roll names every clip the same way and says where each one lives.
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('roll-clip').locator('option')).toHaveText([
    'Clip 2 (Arrangement · bar 1)',
    'Clip 3 (Arrangement · bar 2)',
    'Clip 1 (Grid · Scene 1)',
  ])
  await page.getByTestId('roll-clip').selectOption({ label: 'Clip 2 (Arrangement · bar 1)' })
  await expect(page.getByTestId('roll-clip-name')).toHaveText('Clip 2')
  await expect(page.getByTestId('roll-clip-origin')).toHaveText('Arrangement · bar 1')

  // 5. An arrangement clip is renamed from the roll's title; the new name is the same everywhere.
  await page.getByTestId('roll-clip-name').dblclick()
  await page.getByTestId('roll-clip-rename').fill('Verse')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('roll-clip-name')).toHaveText('Verse')
  expect(await names(page)).toEqual(['Clip 1', 'Clip 3', 'Verse'])
  await expect(page.getByTestId('roll-clip').locator('option').first()).toHaveText('Verse (Arrangement · bar 1)')
  // Clearing the name of a clip that has a default one keeps it.
  await page.getByTestId('roll-clip').selectOption({ label: 'Clip 3 (Arrangement · bar 2)' })
  await page.getByTestId('roll-clip-name').dblclick()
  await page.getByTestId('roll-clip-rename').fill('')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('roll-clip-name')).toHaveText('Clip 3')

  expect(seen.errors).toEqual([])
})
