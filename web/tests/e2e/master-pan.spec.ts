import { expect, test, type Page } from '@playwright/test'
import { engineUp, fakeWebMidi, watch } from './support'

// #412: the master's pan on its strip (typed, dragged, undone) and automated through the "Automate" button, which the
// tracks' strips have too. A real browser with the real engine and project core.

interface Lane {
  target: Record<string, unknown>
}
type Project = { master: { pan: number }; master_automation?: Record<string, Lane>; tracks: Record<string, { automation: Record<string, Lane> }> }

const project = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): { project: Project } }).__faulkView().project)
const trackLanes = async (page: Page) => Object.keys(Object.values((await project(page)).tracks)[0]!.automation).length
const panLanes = async (page: Page) =>
  Object.values((await project(page)).master_automation ?? {}).filter((l) => 'mixer' in l.target && (l.target.mixer as { param: string }).param === 'pan')
    .length

test.beforeEach(async ({ page }) => {
  await fakeWebMidi(page)
})

test('master pan: set, dragged, undone, automated; the tracks have the same Automate buttons', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1600, height: 900 })
  await page.goto('/app')
  await page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
  await engineUp(page)
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkView?: unknown }).__faulkView)) !== 'function',
    'the project view hook exists in the dev server only',
  )
  await page.getByTestId('nav-arrangement').click()
  const arr = page.getByTestId('arrangement')
  await expect(arr).toBeVisible()
  const masterToggle = page.getByTestId('arr-master-row').getByTestId('arr-automation-toggle')
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'false')

  await page.getByTestId('nav-mixer').click()
  const pan = page.getByTestId('mixer-master-pan')
  await expect(pan).toHaveAttribute('aria-valuetext', 'C')

  // A typed value is one command; one undo takes it back, redo gives it again.
  await page.getByTestId('mixer-master-pan-readout').click()
  await page.keyboard.type('L50')
  await page.keyboard.press('Enter')
  await expect(pan).toHaveAttribute('aria-valuetext', 'L50')
  expect((await project(page)).master.pan).toBe(-0.5)
  await page.keyboard.press('Control+z')
  await expect(pan).toHaveAttribute('aria-valuetext', 'C')
  expect((await project(page)).master.pan).toBe(0)
  await page.keyboard.press('Control+Shift+z')
  await expect(pan).toHaveAttribute('aria-valuetext', 'L50')

  // A double click centres; a drag is one history group: one undo takes the whole drag back.
  await pan.dblclick()
  await expect(pan).toHaveAttribute('aria-valuetext', 'C')
  const box = (await pan.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width - 4, box.y + box.height / 2, { steps: 8 })
  await page.mouse.up()
  await expect.poll(async () => (await project(page)).master.pan).toBeGreaterThan(0.5)
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await project(page)).master.pan).toBe(0)

  // Automate: creates the master's pan lane and opens the arrangement with the master unfolded.
  await page.getByTestId('mixer-master-automate-pan').click()
  await expect(arr).toBeVisible()
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'true')
  await expect.poll(() => panLanes(page)).toBe(1)
  // A second click only navigates: no second lane.
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('mixer-master-automate-pan').click()
  await expect(arr).toBeVisible()
  expect(await panLanes(page)).toBe(1)

  // Parity: a track's strip has the same buttons (volume and pan); the lane is created on the track and unfolded.
  await page.getByTestId('nav-mixer').click()
  const strip = page.getByTestId('mixer-strip').first()
  await strip.getByTestId('mixer-automate-pan').click()
  await expect(arr).toBeVisible()
  await expect(page.getByTestId('arr-automation-toggle').first()).toHaveAttribute('aria-expanded', 'true')
  await expect.poll(() => trackLanes(page)).toBe(1)
  await page.getByTestId('nav-mixer').click()
  await strip.getByTestId('mixer-automate-gain').click()
  await expect.poll(() => trackLanes(page)).toBe(2)
  await page.getByTestId('nav-mixer').click()
  await strip.getByTestId('mixer-automate-gain').click()
  await expect(arr).toBeVisible()
  expect(await trackLanes(page)).toBe(2)

  // The track's device knobs have the button too: one lane for the knob, none more on a second click.
  await page.getByTestId('nav-mixer').click()
  await strip.getByTestId('mixer-devices').click()
  const knob = page.getByTestId('bottom-panel').getByTestId('device-param-automate').first()
  await expect(knob).toBeVisible()
  await knob.click()
  await expect(arr).toBeVisible()
  await expect.poll(() => trackLanes(page)).toBe(3)
  await page.getByTestId('nav-mixer').click()
  await strip.getByTestId('mixer-devices').click()
  await knob.click()
  await expect(arr).toBeVisible()
  expect(await trackLanes(page)).toBe(3)

  expect(seen.errors).toEqual([])
})
