import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #195: tap tempo (the button and Alt+T) and the fine tempo steps (arrows, wheel) on the tempo field. Real worklet, engine
// and project core: every change is `transport.set_tempo`; a series of taps (or wheel notches) is one history entry.

async function start(page: Page, lang: 'en' | 'fr' = 'en') {
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

const tempoOf = async (page: Page) => Number(await page.getByTestId('transport-tempo').inputValue())

/** Types a tempo in the field: the baseline the history checks go back to. */
async function setTempo(page: Page, value: string) {
  const tempo = page.getByTestId('transport-tempo')
  await tempo.fill(value)
  await tempo.press('Enter')
  await expect(tempo).toHaveValue(value)
  await tempo.blur()
}

test('tapping the button sets the tempo from the second tap, and one undo takes the series back', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await setTempo(page, '90')
  const tap = page.getByTestId('transport-tap')
  await expect(tap).toBeEnabled()
  await tap.click()
  // One tap measures nothing.
  await expect(page.getByTestId('transport-tempo')).toHaveValue('90')
  for (let i = 0; i < 4; i++) {
    await page.waitForTimeout(500)
    await tap.click()
  }
  // About 120 BPM (500 ms apart, the browser's timers are loose), a multiple of 0.1, announced by the button's live region.
  await expect.poll(() => tempoOf(page)).toBeGreaterThan(105)
  expect(await tempoOf(page)).toBeLessThan(135)
  await expect(page.getByTestId('transport-tapped')).toHaveText(`Tempo ${await tempoOf(page)} BPM`)
  // One undo: the whole series, back to the typed tempo.
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('transport-tempo')).toHaveValue('90')
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('transport-tempo')).toHaveValue('120')
  expect(errors).toEqual([])
})

test('Alt+T taps, in French too, and a pause longer than 2 s starts a new series', async ({ page }) => {
  const { errors } = watch(page)
  await start(page, 'fr')
  await setTempo(page, '90')
  await expect(page.getByTestId('transport-tap')).toHaveText('Tap')
  await page.keyboard.press('Alt+t')
  await page.waitForTimeout(400)
  await page.keyboard.press('Alt+t')
  await expect.poll(() => tempoOf(page)).toBeGreaterThan(130)
  await expect(page.getByTestId('transport-tapped')).toContainText('Tempo ')
  // After more than 2 s the next tap is a first tap again: the tempo stays.
  const kept = await tempoOf(page)
  await page.waitForTimeout(2300)
  await page.keyboard.press('Alt+t')
  await page.waitForTimeout(100)
  expect(await tempoOf(page)).toBe(kept)
  // Two series are two undo entries.
  await page.waitForTimeout(700)
  await page.keyboard.press('Alt+t')
  await expect.poll(() => tempoOf(page)).not.toBe(kept)
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('transport-tempo')).toHaveValue(String(kept))
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('transport-tempo')).toHaveValue('90')
  expect(errors).toEqual([])
})

test('the bare T still plays a note and taps nothing', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await setTempo(page, '90')
  await page.keyboard.press('t')
  await page.waitForTimeout(300)
  await page.keyboard.press('t')
  await page.waitForTimeout(100)
  expect(await tempoOf(page)).toBe(90)
  expect(errors).toEqual([])
})

test('arrows step the tempo by 1 BPM, by 0.1 with Shift or Ctrl, and the wheel steps it too (a run of notches is one entry)', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await setTempo(page, '100')
  const tempo = page.getByTestId('transport-tempo')
  await tempo.focus()
  await tempo.press('ArrowUp')
  await expect(tempo).toHaveValue('101')
  await tempo.press('Shift+ArrowUp')
  await expect(tempo).toHaveValue('101.1')
  await tempo.press('Control+ArrowDown')
  await tempo.press('Control+ArrowDown')
  await expect(tempo).toHaveValue('100.9')
  await tempo.press('ArrowDown')
  await expect(tempo).toHaveValue('100')
  await tempo.blur()

  await tempo.hover()
  await page.mouse.wheel(0, -100)
  await page.mouse.wheel(0, -100)
  await page.mouse.wheel(0, -100)
  await expect(tempo).toHaveValue('103')
  await page.keyboard.down('Shift')
  await page.mouse.wheel(0, 100)
  await page.keyboard.up('Shift')
  await expect(tempo).toHaveValue('102.9')
  // The notches were one series: one undo, back to the last arrow's tempo.
  await page.keyboard.press('Control+z')
  await expect(tempo).toHaveValue('100')
  expect(errors).toEqual([])
})
