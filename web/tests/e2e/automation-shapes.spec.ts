import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #881: the curve of an automation segment, in a real browser with the real worklet, engine and project core. A ramp is drawn
// with each of the five curves through the "Ramp to…" dialog, a point's curve is changed with C and Shift+C and from the
// toolbar's More menu, one undo takes a change back, and the curves come back after a reload (the draft of the project).
// Every change is a command: a refused one would be a console warning, which the test forbids, with every console error.

const BAR = 3840

interface Lane {
  points: Record<string, { value: number; shape: string }>
}
type View = { project: { tracks: Record<string, { automation: Record<string, Lane> }> } }

const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const project = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView().project)
/** The points of the first lane of the first track, as [tick, value, shape]. */
async function lanePoints(page: Page): Promise<[number, number, string][]> {
  const lane = Object.values(Object.values((await project(page)).tracks)[0]!.automation)[0]
  if (!lane) return []
  return Object.entries(lane.points)
    .map(([tick, p]): [number, number, string] => [Number(tick), p.value, p.shape])
    .sort((a, b) => a[0] - b[0])
}
/** The playhead to a bar, through "Go to" (Ctrl+G), as a person does it. */
async function goTo(page: Page, bar: number) {
  await page.keyboard.press('Control+g')
  await expect(page.getByTestId('goto-field')).toBeFocused()
  await page.getByTestId('goto-field').fill(String(bar))
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('goto-dialog')).not.toBeVisible()
  await expect(page.getByTestId('transport-position')).toContainText(`${bar}.1`)
}
const point = (page: Page, tick: number) => page.locator(`[data-testid=arr-point][data-tick="${tick}"]`)

async function open(page: Page) {
  await page.setViewportSize({ width: 1600, height: 900 })
  await page.goto('/app')
  await page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
  await engineUp(page)
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('arrangement')).toBeVisible()
  const stage = page.getByTestId('arr-stage')
  await expect.poll(async () => Number(await stage.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
}

test('automation curves (#881): a ramp of each curve, a point changed with C, kept after a reload', async ({ page }) => {
  test.setTimeout(180_000)
  const seen = watch(page)
  const warnings: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'warning') warnings.push(message.text())
  })
  await open(page)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  // The volume lane of the first track.
  const toggle = page.getByTestId('arr-automation-toggle').first()
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click()
  await page.getByTestId('arr-add-lane').first().selectOption({ label: 'Volume' })
  const head = page.getByTestId('arr-lane').first()
  await expect(head).toBeVisible()
  const said = page.getByTestId('arr-said')

  // A first point at bar 2, -6 dB: the playhead there, I, the value, Enter.
  await head.getByTestId('arr-lane-point').focus()
  await goTo(page, 2)
  await head.getByTestId('arr-lane-point').focus()
  await page.keyboard.press('i')
  await expect(page.getByTestId('value-input')).toBeFocused()
  await page.keyboard.type('-6')
  await page.keyboard.press('Enter')
  await expect(point(page, BAR)).toBeFocused()

  // Five ramps of a bar each, one per curve: from the playhead (on the last point) to the next bar, -24 dB and -6 dB in turn.
  const curves = ['linear', 'step', 'ease_in', 'ease_out', 'smooth'] as const
  const names = ['Linear', 'Step', 'Slow then fast', 'Fast then slow', 'S-curve']
  for (const [k, shape] of curves.entries()) {
    const from = (k + 1) * BAR
    await point(page, from).focus()
    await goTo(page, k + 2)
    await point(page, from).focus()
    await page.keyboard.press('r')
    const dialog = page.getByTestId('ramp-dialog')
    await expect(dialog).toBeVisible()
    await expect(page.getByTestId('ramp-position')).toBeFocused()
    // The curve is chosen in the dialog, linear unless changed; the five are offered by name.
    const select = page.getByTestId('ramp-curve')
    await expect(select).toHaveValue('linear')
    await expect(select.locator('option')).toHaveText(names)
    await page.keyboard.type(String(k + 3))
    await page.keyboard.press('Tab')
    await page.keyboard.press('Control+a')
    await page.keyboard.type(k % 2 === 0 ? '-24' : '-6')
    await select.selectOption(shape)
    await page.getByTestId('ramp-submit').click()
    await expect(dialog).not.toBeVisible()
    await expect(point(page, from + BAR)).toBeFocused()
    expect((await lanePoints(page)).find((p) => p[0] === from)?.[2], shape).toBe(shape)
  }
  const ramped = await lanePoints(page)
  expect(ramped.map((p) => p[2])).toEqual([...curves, 'linear'])
  expect(ramped.map((p) => p[0])).toEqual([1, 2, 3, 4, 5, 6].map((b) => b * BAR))

  // The point's cell names its curve.
  await expect(point(page, 3 * BAR)).toHaveAttribute('aria-label', /slow then fast/)

  // C on the first point (linear, reached with Home, which selects it): the next curve, said; again; Shift+C goes back. One `points.update` each.
  await expect(point(page, 6 * BAR)).toBeFocused()
  await page.keyboard.press('Home')
  await expect(point(page, BAR)).toBeFocused()
  await expect(point(page, BAR)).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('c')
  await expect(said).toHaveText(/^Curve: step/)
  await page.keyboard.press('c')
  await expect(said).toHaveText(/^Curve: slow then fast/)
  expect((await lanePoints(page))[0]![2]).toBe('ease_in')
  await page.keyboard.press('Shift+C')
  await expect(said).toHaveText(/^Curve: step/)
  expect((await lanePoints(page))[0]).toEqual([BAR, -6, 'step'])
  // From the toolbar's More menu, the selected point (the focused one) becomes an S-curve; one undo takes it back.
  await page.getByTestId('arr-more').click()
  await expect(page.getByTestId('ctx-more-curve-smooth')).toBeEnabled()
  await page.getByTestId('ctx-more-curve-smooth').click()
  const first = async () => (await lanePoints(page))[0]![2]
  await expect.poll(first).toBe('smooth')
  await page.keyboard.press('Control+z')
  await expect.poll(first).toBe('step')
  await page.keyboard.press('Control+Shift+z')
  await expect.poll(first).toBe('smooth')

  // The point's own menu (a right click on it) offers the five curves: fast then slow on the point of bar 3.
  const cell = (await point(page, 2 * BAR).boundingBox())!
  await page.mouse.click(cell.x + cell.width / 2, cell.y + cell.height / 2, { button: 'right' })
  await expect(page.getByTestId('context-menu')).toBeVisible()
  for (const shape of curves) await expect(page.getByTestId(`point-menu-${shape}`)).toBeVisible()
  await page.getByTestId('point-menu-ease_out').click()
  await expect.poll(async () => (await lanePoints(page)).find((p) => p[0] === 2 * BAR)?.[2]).toBe('ease_out')

  // A click of the Draw tool inside the slow-then-fast segment (bars 4 to 5) adds a point of that curve, as a key would.
  const a = (await point(page, 3 * BAR).boundingBox())!
  const b = (await point(page, 4 * BAR).boundingBox())!
  const row = (await head.boundingBox())!
  await page.getByTestId('arr-tool-draw').click()
  await page.mouse.click((a.x + b.x) / 2 + a.width / 2, row.y + row.height / 2)
  await expect.poll(async () => (await lanePoints(page)).length).toBe(7)
  const added = (await lanePoints(page)).find((p) => p[0] > 3 * BAR && p[0] < 4 * BAR)
  expect(added?.[2]).toBe('ease_in')
  await page.getByTestId('arr-tool-select').click()
  const before = await lanePoints(page)

  // Saved with the project (the draft), found again on reopening.
  await expect(page.getByTestId('save-state')).toHaveText(/^Draft saved at/, { timeout: 20_000 })
  await page.reload()
  await open(page)
  await expect.poll(() => lanePoints(page)).toEqual(before)

  expect(warnings).toEqual([])
  expect(seen.errors).toEqual([])
})
