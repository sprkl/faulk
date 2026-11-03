import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #612: the edges of a clip show resize handles when the pointer is over the clip (stronger on the edge itself), a short
// clip keeps a comfortable grab zone, and a move or a resize shows a label near the pointer (position or length, the snap
// in force, the key that frees it) that goes away on release. One resize is one history entry. Alt+Shift+arrows resize
// the end from the keyboard.

const RULER = 28
const LANE = 44
const BAR = 3840

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const arr = (page: Page) => page.getByTestId('arrangement')
const stage = (page: Page) => page.getByTestId('arr-stage')
const blocks = async (page: Page): Promise<{ id: string; start: number; length: number }[]> => JSON.parse((await arr(page).getAttribute('data-blocks')) ?? '[]')

async function spot(page: Page) {
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  return { x: (tick: number) => box.x + tick * ppt, y: box.y + RULER + LANE / 2, ppt, box }
}

/** The colour of a canvas pixel of the stage's grid, at a page point. */
const pixel = (page: Page, x: number, y: number) =>
  page.evaluate(
    ([px, py]) => {
      const canvas = document.querySelector('[data-testid="arr-stage"] [data-testid="timegrid"] canvas') as HTMLCanvasElement
      const box = canvas.getBoundingClientRect()
      const d = canvas
        .getContext('2d')!
        .getImageData(Math.round((px! - box.left) * (canvas.width / box.width)), Math.round((py! - box.top) * (canvas.height / box.height)), 1, 1).data
      return [d[0], d[1], d[2]]
    },
    [x, y],
  )

for (const scheme of ['light', 'dark'] as const) {
  test(`arrangement: clip edge handles, drag label and keyboard resize (${scheme})`, async ({ page }) => {
    test.setTimeout(120_000)
    const seen = watch(page)
    await page.setViewportSize({ width: 1600, height: 900 })
    await page.goto('/app')
    await startAudio(page)
    await engineUp(page)
    // The DAW opens in its dark theme; the light tokens are put on the page, and a resize makes the grid read the colours again.
    await page.evaluate((theme) => document.documentElement.setAttribute('data-theme', theme), scheme)
    await page.setViewportSize({ width: 1601, height: 900 })
    await page.getByTestId('panel-piano-roll').click()
    await page.getByTestId('roll-create-clip').click()
    await page.getByTestId('nav-arrangement').click()
    await expect(arr(page)).toHaveAttribute('data-block-count', '1')
    await expect.poll(async () => Number(await stage(page).getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
    await page.getByTestId('arr-snap').selectOption('beat')
    const g = await spot(page)

    // Nothing is shown with the pointer away; over the body both ends are faint; on the edge that end is strong.
    await page.mouse.move(g.x(BAR * 3), g.y)
    await expect(stage(page)).not.toHaveAttribute('data-handle', /.*/)
    const quiet = await pixel(page, g.x(0) + 5, g.y)
    await page.mouse.move(g.x(BAR / 2), g.y)
    await expect(stage(page)).toHaveAttribute('data-handle', 'body')
    const faint = await pixel(page, g.x(0) + 5, g.y)
    expect(faint).not.toEqual(quiet)
    await page.mouse.move(g.x(BAR) - 3, g.y)
    await expect(stage(page)).toHaveAttribute('data-handle', 'end')
    await expect(stage(page)).toHaveAttribute('data-cursor', 'resize')
    const strong = await pixel(page, g.x(BAR) - 5, g.y)
    const faintEnd = await pixel(page, g.x(0) + 5, g.y)
    expect(strong).not.toEqual(faintEnd)
    if (process.env.FAULK_E2E_SHOTS)
      await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/handles-${scheme}.png`, clip: { x: 270, y: 50, width: 400, height: 110 } })

    // Resize with the mouse: the label shows the length and the snap while dragging, and goes on release.
    await expect(page.getByTestId('gesture-label')).toHaveCount(0)
    await page.mouse.down()
    await page.mouse.move(g.x(BAR + 960), g.y, { steps: 5 })
    const label = page.getByTestId('gesture-label')
    await expect(label).toBeVisible()
    await expect(label).toContainText(/Length 1 bar, 1 beat|Longueur 1 mesure, 1 temps/)
    await expect(label).toContainText(/Snap: Beat|Magnétisme : Temps/)
    await expect(label).toContainText(/Ctrl|⌘/)
    if (process.env.FAULK_E2E_SHOTS)
      await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/label-${scheme}.png`, clip: { x: 270, y: 50, width: 700, height: 160 } })
    // With Ctrl held the snap is set aside, and the label says so.
    await page.keyboard.down('Control')
    await page.mouse.move(g.x(BAR + 1000), g.y, { steps: 2 })
    await expect(label).toContainText(/Snap: Free|Magnétisme : Libre/)
    await page.keyboard.up('Control')
    await page.mouse.move(g.x(BAR + 960), g.y, { steps: 2 })
    await page.mouse.up()
    await expect(label).toHaveCount(0)
    await expect.poll(async () => (await blocks(page))[0]!.length).toBe(BAR + 960)

    // One undo takes the whole resize back.
    await page.keyboard.press('Control+z')
    await expect.poll(async () => (await blocks(page))[0]!.length).toBe(BAR)

    // A move shows the position.
    await page.mouse.move(g.x(BAR / 2), g.y)
    await page.mouse.down()
    await page.mouse.move(g.x(BAR / 2 + 960), g.y, { steps: 5 })
    await expect(label).toContainText(/Bar 1, beat 2|Mesure 1, temps 2/)
    await page.mouse.up()
    await expect(label).toHaveCount(0)
    await expect.poll(async () => (await blocks(page))[0]!.start).toBe(960)

    // Keyboard: Alt+Shift+Right pushes the end by a beat, Alt+Shift+Left brings it back.
    await page.getByTestId('arr-item').first().focus()
    await page.keyboard.press('Alt+Shift+ArrowRight')
    await expect.poll(async () => (await blocks(page))[0]!.length).toBe(BAR + 960)
    await page.keyboard.press('Alt+Shift+ArrowLeft')
    await expect.poll(async () => (await blocks(page))[0]!.length).toBe(BAR)

    expect(seen.errors).toEqual([])
  })
}

test('piano roll: a note end shows its handle, and a resize shows the length and the snap, freed by Ctrl', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await engineUp(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await page.getByTestId('roll-snap').selectOption('beat')
  const stage = page.getByTestId('roll-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await page.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await stage.getAttribute('data-scroll-y'))
  const x = (tick: number) => box.x + tick * ppt
  const y = box.y + RULER + (127 - 60) * 16 - scrollY + 8

  // A click draws a note of one beat; the pointer on its end shows the resize cursor.
  await page.mouse.click(x(10), y)
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '1')
  await page.mouse.move(x(960) - 3, y)
  await expect(stage).toHaveAttribute('data-cursor', 'resize')

  const label = page.getByTestId('gesture-label')
  await expect(label).toHaveCount(0)
  await page.mouse.down()
  await page.mouse.move(x(1920), y, { steps: 5 })
  await expect(label).toContainText(/Length 2 beats|Longueur 2 temps/)
  await expect(label).toContainText(/Snap: Beat|Magnétisme : Temps/)
  await page.keyboard.down('Control')
  await page.mouse.move(x(1900), y, { steps: 2 })
  await expect(label).toContainText(/Snap: Free|Magnétisme : Libre/)
  await page.keyboard.up('Control')
  await page.mouse.move(x(1920), y, { steps: 2 })
  await page.mouse.up()
  await expect(label).toHaveCount(0)

  // One undo takes the resize back, and the note stays.
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '1')
  expect(seen.errors).toEqual([])
})
