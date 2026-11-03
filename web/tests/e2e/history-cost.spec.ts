import { expect, test } from '@playwright/test'
import { engineUp, watch } from './support'

// #432: with a long history and the tab open, every edit re-read the whole history and redrew every row.
// Measured on 10 000 entries (Chromium, a 100-step history group, one frame per step): the read
// (`pw_history`) costs a median 3.5 ms, but drawing 10 000 rows cost a median 217 ms per step (p95 273 ms)
// against 17 ms with the tab closed. The panel now draws 200 rows and adds 200 per click on "older entries".
// The bound is loose (3 frames) so a slow CI machine does not flake; the undrawn panel is several times over it.

type W = { __faulkRun(c: object, a?: string): void }

const SEEDED = 10_000
const PAGE = 200
const MEDIAN_FRAME_MS = 50

test('history panel: a 10 000-entry history draws a page of rows and an edit stays within a few frames', async ({ page }) => {
  test.setTimeout(180_000)
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1400, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  const hook = await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
  test.skip(hook !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.evaluate((n) => {
    const w = window as unknown as W
    for (let i = 0; i < n; i++) w.__faulkRun({ type: 'project.rename', name: `p${i}` })
  }, SEEDED)
  await page.getByTestId('panel-history').click()
  const rows = page.getByTestId('history-entry')
  await expect(rows).toHaveCount(PAGE)

  // A drag: one history group, many edits, one frame each, tab open.
  const frames = await page.evaluate(async () => {
    const w = window as unknown as W
    const frame = () => new Promise<void>((r) => requestAnimationFrame(() => r()))
    const times: number[] = []
    w.__faulkRun({ type: 'history.begin_group' })
    for (let i = 0; i < 60; i++) {
      const t0 = performance.now()
      w.__faulkRun({ type: 'project.rename', name: `drag${i}` })
      await frame()
      times.push(performance.now() - t0)
    }
    w.__faulkRun({ type: 'history.end_group' })
    return times.sort((a, b) => a - b)
  })
  console.log(`history panel open, ${SEEDED} entries: median ${frames[30]!.toFixed(1)} ms, p95 ${frames[57]!.toFixed(1)} ms`)
  expect(frames[30]!).toBeLessThan(MEDIAN_FRAME_MS)
  // The group is one entry more, drawn first; the rows are still one page.
  await expect(rows).toHaveCount(PAGE)
  await expect(rows.first()).toHaveAttribute('data-name', 'project.rename')

  // The older entries come a page at a time, until none is left.
  await page.getByTestId('history-older').click()
  await expect(rows).toHaveCount(2 * PAGE)
  const left = Number(/\d+/.exec((await page.getByTestId('history-older').textContent()) ?? '')?.[0])
  expect(left).toBeGreaterThan(SEEDED - 2 * PAGE)
  expect(left).toBeLessThan(SEEDED + 10 - 2 * PAGE)
  expect(errors).toEqual([])
})
