import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #470: musical typing. The indicator is always shown, one key (the one left of 1) toggles it, and with it
// off the bare letters play nothing. The piano roll opens on the notes of its clip. Real worklet and engine.

const trackPeak = async (page: Page) => Number(((await page.getByTestId('feedback').getAttribute('data-track-peaks')) ?? '').split(',')[0])
const masterPeak = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
const mode = (page: Page) => page.getByTestId('typing-mode')

async function start(page: Page) {
  await page.setViewportSize({ width: 1920, height: 1080 })
  await page.goto('/app')
  // The indicator shows before the audio starts.
  await expect(mode(page)).toHaveText(/Typing: notes|Saisie : notes/)
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await expect(page.getByTestId('track-inspector')).toBeVisible()
}

test('musical typing: the letters play notes only while the mode is on, one key toggles it', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await expect(mode(page)).toHaveAttribute('data-mode', 'notes')

  await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)
  await page.keyboard.down('KeyA')
  await expect.poll(() => trackPeak(page), { message: 'A sounds while typing plays notes' }).toBeGreaterThan(0.05)
  await page.keyboard.up('KeyA')
  await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)

  // The key left of 1 switches to shortcuts: the letter plays nothing.
  await page.keyboard.press('Backquote')
  await expect(mode(page)).toHaveAttribute('data-mode', 'shortcuts')
  await expect(mode(page)).toHaveText(/Typing: shortcuts|Saisie : raccourcis/)
  await page.keyboard.down('KeyA')
  await page.waitForTimeout(400)
  expect(await trackPeak(page)).toBeLessThan(0.001)
  await page.keyboard.up('KeyA')

  // Space stays bare: it still plays the transport.
  await page.keyboard.press('Space')
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  await page.keyboard.press('Space')
  await expect(page.getByTestId('feedback')).not.toHaveAttribute('data-transport', 'playing')

  // The indicator is a button too, and back on, the letter plays again.
  await mode(page).click()
  await expect(mode(page)).toHaveAttribute('data-mode', 'notes')
  await page.keyboard.down('KeyA')
  await expect.poll(() => trackPeak(page), { message: 'A sounds again' }).toBeGreaterThan(0.05)
  await page.keyboard.up('KeyA')

  expect(errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

test('the piano roll shows a drawn note in view, and the other tracks toggle flips', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  const stage = page.getByTestId('roll-stage')
  await expect(stage).toBeVisible()
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)

  // A note at the middle row, then scroll far away from it.
  const box = (await stage.boundingBox())!
  const ppt = Number(await page.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await stage.getAttribute('data-scroll-y'))
  await page.mouse.click(box.x + 1000 * ppt, box.y + 28 + (127 - 60) * 16 - scrollY + 8)
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '1')
  // The row of the note is in the stage. (Opening on a clip's notes is `initialScrollY`, unit-tested in roll.test.ts;
  // a clip left and reopened keeps its remembered place since #475.)
  const top = Number(await stage.getAttribute('data-scroll-y'))
  const height = (await stage.boundingBox())!.height
  const rowTop = (127 - 60) * 16 - top + 28
  expect(rowTop).toBeGreaterThanOrEqual(28)
  expect(rowTop + 16).toBeLessThanOrEqual(height)

  // The row names and the toggle of the other tracks' notes are there.
  await expect(page.getByTestId('roll-others')).toHaveAttribute('aria-pressed', 'true')
  await page.getByTestId('roll-others').click()
  await expect(page.getByTestId('roll-others')).toHaveAttribute('aria-pressed', 'false')
  expect(errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})
