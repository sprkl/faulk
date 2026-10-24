import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// TR-4 (#62): the transport bar in a real browser. Real worklet, engine and project core: every
// control dispatches a command, the bar shows what the project and the engine report back.
// Screenshots go to the folder named by FAULK_E2E_SHOTS, when set.

const TEXTS = {
  en: { play: 'Play', stop: 'Stop', rewind: 'Back to start', record: 'Record', loop: 'Loop', metronome: 'Metronome', tempo: 'Tempo (BPM)', numerator: 'Beats per bar', denominator: 'Beat unit', quantize: 'Launch quantization', bar: '1 bar' },
  fr: { play: 'Lecture', stop: 'Stop', rewind: 'Retour au début', record: 'Enregistrer', loop: 'Boucle', metronome: 'Métronome', tempo: 'Tempo (BPM)', numerator: 'Temps par mesure', denominator: 'Valeur du temps', quantize: 'Quantification de lancement', bar: '1 mesure' },
} as const

const control = (page: Page, name: string) => page.getByTestId('transport').getByRole('button', { name, exact: true })
const feedback = (page: Page) => page.getByTestId('feedback')
const tick = async (page: Page) => Number(await feedback(page).getAttribute('data-tick'))

/** The highest output peak the page showed during `ms`: a click is a few milliseconds, so every frame is read. */
const loudest = (page: Page, ms: number): Promise<number> =>
  page.evaluate(
    (duration) =>
      new Promise<number>((done) => {
        const el = document.querySelector('[data-testid=feedback]')!
        const end = performance.now() + duration
        let max = 0
        const frame = () => {
          max = Math.max(max, Number(el.getAttribute('data-master-peak')))
          if (performance.now() < end) requestAnimationFrame(frame)
          else done(max)
        }
        frame()
      }),
    ms,
  )

/** The playhead (ticks) as the page showed it on every frame during `ms`. */
const playheadTrace = (page: Page, ms: number): Promise<number[]> =>
  page.evaluate(
    (duration) =>
      new Promise<number[]>((done) => {
        const el = document.querySelector('[data-testid=feedback]')!
        const end = performance.now() + duration
        const seen: number[] = []
        const frame = () => {
          seen.push(Number(el.getAttribute('data-tick')))
          if (performance.now() < end) requestAnimationFrame(frame)
          else done(seen)
        }
        frame()
      }),
    ms,
  )

async function horizontalOverflow(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const bad: string[] = []
    for (const selector of ['header.top', '[data-testid=transport]']) {
      for (const el of document.querySelectorAll(selector)) {
        if (el.scrollWidth > el.clientWidth + 1) bad.push(`${selector}: scrollWidth ${el.scrollWidth} > ${el.clientWidth}`)
      }
    }
    if (document.documentElement.scrollWidth > innerWidth) bad.push('page: scrolls sideways')
    return bad
  })
}

for (const lang of ['en', 'fr'] as const) {
  for (const width of [1280, 1920]) {
    test(`transport, ${lang}, ${width} px: play, stop, tempo, signature, loop, metronome`, async ({ page }, info) => {
      const x = TEXTS[lang]
      const { errors } = watch(page)
      await page.setViewportSize({ width, height: 900 })
      await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
      await page.goto('/app')

      const tempo = page.getByTestId('transport-tempo')
      const position = page.getByTestId('transport-position')

      // Before the audio starts the controls are off, and record stays off after.
      await expect(control(page, x.play)).toBeDisabled()
      await expect(tempo).toBeDisabled()
      await page.getByTestId('engine-state').getByRole('button').click()
      await expect(page.getByTestId('engine-mode')).toBeVisible()
      await expect(control(page, x.play)).toBeEnabled()
      await expect(control(page, x.record)).toBeDisabled()
      await expect(page.getByTestId('transport-quantize')).toBeEnabled()
      await expect(page.getByTestId('transport-quantize')).toHaveValue('bar')
      await expect(page.getByTestId('transport-quantize').locator('option:checked')).toHaveText(x.bar)

      // Play: the engine reports playing, the position moves; Stop keeps the position; play again restarts from the last seek.
      await expect(position).toHaveText('1.1.000')
      await expect(page.getByTestId('transport-time')).toHaveText('00:00.000')
      await control(page, x.play).click()
      await expect(feedback(page)).toHaveAttribute('data-transport', 'playing')
      await expect(control(page, x.play)).toHaveAttribute('aria-pressed', 'true')
      await expect.poll(() => tick(page)).toBeGreaterThan(960)
      await expect(position).not.toHaveText('1.1.000')
      await control(page, x.stop).click()
      await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
      await expect(control(page, x.play)).toHaveAttribute('aria-pressed', 'false')
      const stopped = await tick(page)
      expect(stopped).toBeGreaterThan(960)
      await page.waitForTimeout(300)
      expect(await tick(page)).toBe(stopped)

      // Back to start: a seek to tick 0.
      await control(page, x.rewind).click()
      await expect.poll(() => tick(page)).toBe(0)
      await expect(position).toHaveText('1.1.000')

      // Space plays and stops from anywhere but a field or a focused button.
      await page.locator('[data-testid=view]').click({ position: { x: 5, y: 5 } })
      await page.keyboard.press('Space')
      await expect(feedback(page)).toHaveAttribute('data-transport', 'playing')
      await page.keyboard.press('Space')
      await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
      await control(page, x.rewind).click()
      // After a mouse click on Play, Space stops (the button does not keep the focus).
      await control(page, x.play).click()
      await expect(feedback(page)).toHaveAttribute('data-transport', 'playing')
      await page.keyboard.press('Space')
      await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
      await control(page, x.rewind).click()

      // The tempo reaches the engine: the playhead runs twice as fast at 240 BPM as at 120.
      const rate = async () => {
        await control(page, x.rewind).click()
        await control(page, x.play).click()
        await expect.poll(() => tick(page)).toBeGreaterThan(0)
        const from = await tick(page)
        await page.waitForTimeout(1000)
        const gained = (await tick(page)) - from
        await control(page, x.stop).click()
        await control(page, x.rewind).click()
        return gained
      }
      const slow = await rate()
      await tempo.fill('240')
      await tempo.press('Enter')
      const fast = await rate()
      expect(fast / slow).toBeGreaterThan(1.7)
      expect(fast / slow).toBeLessThan(2.3)
      await tempo.focus()
      await page.keyboard.press('Escape')
      await page.keyboard.press('Control+z')
      await expect(tempo).toHaveValue('120')

      // Tempo: Enter applies; an invalid text or Escape gives the edit up; blur applies once; undo takes it back.
      await expect(tempo).toHaveValue('120')
      await tempo.fill('140')
      await tempo.press('Enter')
      await expect(tempo).toHaveValue('140')
      await tempo.fill('1000')
      await tempo.press('Enter')
      await expect(tempo).toHaveValue('140')
      await tempo.fill('abc')
      await tempo.blur()
      await expect(tempo).toHaveValue('140')
      await tempo.fill('99')
      await tempo.press('Escape')
      await expect(tempo).toHaveValue('140')
      await tempo.fill('150,5')
      await tempo.blur()
      await expect(tempo).toHaveValue('150.5')
      await tempo.focus()
      await tempo.press('ArrowUp')
      await expect(tempo).toHaveValue('151')
      await tempo.blur()
      await page.keyboard.press('Control+z')
      await expect(tempo).toHaveValue('150.5')
      await page.keyboard.press('Control+z')
      await expect(tempo).toHaveValue('140')
      await page.keyboard.press('Control+z')
      await expect(tempo).toHaveValue('120')

      // Time signature: numerator field and denominator list; the position follows.
      const numerator = page.getByTestId('transport-numerator')
      const denominator = page.getByTestId('transport-denominator')
      await expect(numerator).toHaveValue('4')
      await numerator.fill('3')
      await numerator.press('Enter')
      await denominator.selectOption('8')
      await expect(numerator).toHaveValue('3')
      await expect(denominator).toHaveValue('8')
      await numerator.fill('33')
      await numerator.blur()
      await expect(numerator).toHaveValue('3')
      // The command hook exists on the dev server only: a seek to one bar of 3/8 reads 2.1.000.
      if (info.project.name === 'dev') {
        await page.evaluate(() => (window as unknown as { __faulkRun(c: object): void }).__faulkRun({ type: 'transport.seek', tick: 1440 }))
        await expect(position).toHaveText('2.1.000')
      }
      await control(page, x.rewind).click()
      await page.keyboard.press('Control+z')
      await page.keyboard.press('Control+z')
      await expect(numerator).toHaveValue('4')
      await expect(denominator).toHaveValue('4')

      // Launch quantization: the seven values, each choice is one command, undo takes it back.
      const quantize = page.getByTestId('transport-quantize')
      await expect(quantize.locator('option')).toHaveCount(7)
      await quantize.selectOption('bars_4')
      await expect(quantize).toHaveValue('bars_4')
      await quantize.selectOption('none')
      await expect(quantize).toHaveValue('none')
      await quantize.blur()
      await page.keyboard.press('Control+z')
      await expect(quantize).toHaveValue('bars_4')
      await page.keyboard.press('Control+z')
      await expect(quantize).toHaveValue('bar')

      // Loop: the first press gives the project a region and turns the loop on, in one undo step; the second turns it off.
      const loop = control(page, x.loop)
      await expect(loop).toHaveAttribute('aria-pressed', 'false')
      await loop.click()
      await expect(loop).toHaveAttribute('aria-pressed', 'true')
      await loop.click()
      await expect(loop).toHaveAttribute('aria-pressed', 'false')
      await loop.click()
      await expect(loop).toHaveAttribute('aria-pressed', 'true')
      // The loop wraps the playhead inside its four bars.
      await tempo.fill('400')
      await tempo.press('Enter')
      await control(page, x.play).click()
      await expect(feedback(page)).toHaveAttribute('data-transport', 'playing')
      const seen = await playheadTrace(page, 4500)
      // It climbs near the end of the region (15360 ticks), then falls back: it wrapped, and never passed the end.
      expect(Math.max(...seen)).toBeGreaterThan(12000)
      expect(Math.max(...seen)).toBeLessThan(15360 + 960)
      expect(seen.some((t, i) => i > 0 && t < seen[i - 1] - 5000)).toBe(true)
      await control(page, x.stop).click()
      await control(page, x.rewind).click()
      await page.keyboard.press('Escape')

      // Metronome: on, off.
      const metronome = control(page, x.metronome)
      await expect(metronome).toHaveAttribute('aria-pressed', 'false')
      await metronome.click()
      await expect(metronome).toHaveAttribute('aria-pressed', 'true')
      // The click reaches the output: the project has no clip, so any level comes from the metronome.
      await tempo.fill('120')
      await tempo.press('Enter')
      await control(page, x.play).click()
      expect(await loudest(page, 1500)).toBeGreaterThan(0.05)
      await control(page, x.stop).click()
      await metronome.click()
      await expect(metronome).toHaveAttribute('aria-pressed', 'false')
      await control(page, x.play).click()
      expect(await loudest(page, 1500)).toBeLessThan(0.001)
      await control(page, x.stop).click()

      expect(await horizontalOverflow(page)).toEqual([])
      const shots = process.env.FAULK_E2E_SHOTS
      if (shots) {
        mkdirSync(shots, { recursive: true })
        await page.screenshot({ path: `${shots}/transport-${lang}-${width}.png`, clip: { x: 0, y: 0, width, height: 120 } })
      }
      expect(errors).toEqual([])
    })
  }
}

test('the space bar presses a focused control and never toggles the transport behind it', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  // Space on a focused button presses that button, it does not also toggle the transport.
  // A Tab press first: the last input was the mouse, and a focus that follows a mouse press is the mouse's (#332).
  await page.keyboard.press('Tab')
  await control(page, 'Stop').focus()
  await page.keyboard.press('Space')
  // A play command would show as `playing` within a report or two: wait well past that.
  await page.waitForTimeout(400)
  await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
  await page.getByTestId('transport-tempo').focus()
  await page.keyboard.press('Space')
  await page.waitForTimeout(400)
  await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
  expect(errors).toEqual([])
})

test('double clicks and a reload during playback leave the bar consistent', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  // A double click on Play plays once; one on Stop and on the loop button end in a consistent state.
  await control(page, 'Play').dblclick()
  await expect(feedback(page)).toHaveAttribute('data-transport', 'playing')
  await control(page, 'Stop').dblclick()
  await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
  await control(page, 'Loop').dblclick()
  await expect(control(page, 'Loop')).toHaveAttribute('aria-pressed', 'false')
  // Reload while playing: the page comes back stopped, with the controls off until audio starts.
  await control(page, 'Play').click()
  await expect(feedback(page)).toHaveAttribute('data-transport', 'playing')
  await page.reload()
  await expect(control(page, 'Play')).toBeDisabled()
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
  await expect(page.getByTestId('transport-position')).toHaveText('1.1.000')
  expect(errors).toEqual([])
})

test('leaving the tempo field untouched does not round a tempo with more decimals', async ({ page }, info) => {
  test.skip(info.project.name !== 'dev', 'the command hook exists on the dev server only')
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  const tempo = page.getByTestId('transport-tempo')
  await page.evaluate(() => (window as unknown as { __faulkRun(c: object): void }).__faulkRun({ type: 'transport.set_tempo', bpm: 120.456 }))
  await expect(tempo).toHaveValue('120.46')
  await tempo.focus()
  await tempo.blur()
  // One undo step only: the one that set 120.456.
  await page.keyboard.press('Control+z')
  await expect(tempo).toHaveValue('120')
  expect(errors).toEqual([])
})
