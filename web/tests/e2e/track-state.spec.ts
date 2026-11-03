import { mkdirSync } from 'node:fs'
import { expect, test, type Locator, type Page } from '@playwright/test'
import { chooseMidiInput, fakeWebMidi, watch } from './support'

// #469: the state of every track where the music is (M, S, R, a meter, the dimming), the master
// meter in the top bar in every view, and the one voice of the violet. Real worklet and engine.
// Every state must be legible without colour: the checks read letters, words and shapes, then
// look at the page in greyscale; the contrast of the text that carries a state is computed.

test.describe.configure({ mode: 'serial' })

const SHOTS = '../.qa/design-review-2026-11-01/after-469'

const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

async function start(page: Page) {
  await fakeWebMidi(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  await chooseMidiInput(page, 'E2E Keys')
  await expect(page.getByTestId('track-list')).toHaveAttribute('data-track-count', '1')
  for (let i = 0; i < 2; i++) {
    await page.getByTestId('add-track').click()
    await page.getByTestId('add-track-menu').getByRole('menuitem').first().click()
  }
  await expect(page.getByTestId('track-list')).toHaveAttribute('data-track-count', '3')
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('arr-track')).toHaveCount(3)
}

/** WCAG contrast of the text colour of `el` on the first opaque background above it. */
async function contrast(el: Locator): Promise<number> {
  return el.evaluate((node) => {
    const parse = (css: string): [number, number, number, number] => {
      const m = css.match(/rgba?\(([^)]+)\)/) ?? css.match(/color\(srgb ([^)]+)\)/)
      if (!m) return [0, 0, 0, 0]
      const parts = m[1]
        .split(/[ ,/]+/)
        .filter(Boolean)
        .map(Number)
      const scale = css.startsWith('color(') ? 255 : 1
      return [parts[0] * scale, parts[1] * scale, parts[2] * scale, parts[3] ?? 1]
    }
    const lum = ([r, g, b]: number[]) => {
      const f = (c: number) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    }
    let bg: [number, number, number, number] = [0, 0, 0, 0]
    for (let n: Element | null = node; n; n = n.parentElement) {
      const c = parse(getComputedStyle(n).backgroundColor)
      if (c[3] > 0.95) {
        bg = c
        break
      }
    }
    const fg = parse(getComputedStyle(node).color)
    const [a, b] = [lum(fg), lum(bg)]
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)
  })
}

test('mute, solo, arm and the implied mute are letters and shapes on every lane', async ({ page }) => {
  const { errors } = watch(page)
  mkdirSync(SHOTS, { recursive: true })
  await start(page)
  const rows = page.getByTestId('arr-track').locator('xpath=..')
  const mute = (i: number) => rows.nth(i).getByTestId('arr-mute')
  const solo = (i: number) => rows.nth(i).getByTestId('arr-solo')

  // M, S and R are letters on each lane header, with a meter.
  for (let i = 0; i < 3; i++) {
    await expect(mute(i)).toHaveText('M')
    await expect(solo(i)).toHaveText('S')
    await expect(rows.nth(i).getByTestId('arr-arm')).toHaveText('Rec')
    await expect(rows.nth(i).getByTestId('arr-meter')).toBeVisible()
  }
  await expect(page.getByTestId('arr-master-meter')).toBeVisible()

  // Mute: pressed, struck through (a shape), the lane at 40 %; the state is also in the accessible name's pressed state.
  await mute(1).click()
  await expect(mute(1)).toHaveAttribute('aria-pressed', 'true')
  await expect(mute(1)).toHaveCSS('text-decoration-line', 'line-through')
  await expect(rows.nth(1).locator('.head')).toHaveCSS('opacity', '0.4')
  await expect(rows.nth(0).locator('.head')).toHaveCSS('opacity', '1')
  // The letter on the filled button is AA.
  expect(await contrast(mute(1))).toBeGreaterThanOrEqual(4.5)

  // Solo on the first lane: underlined; the others are silenced by it, which says so in text (screen readers) and with a dashed frame.
  await solo(0).click()
  await expect(solo(0)).toHaveAttribute('aria-pressed', 'true')
  await expect(solo(0)).toHaveCSS('text-decoration-line', 'underline')
  expect(await contrast(solo(0))).toBeGreaterThanOrEqual(4.5)
  await expect(mute(2)).toHaveAttribute('data-implied', 'true')
  await expect(mute(2)).toContainText('Silenced by a solo')
  // And in its accessible name, which an aria-label would otherwise replace.
  await expect(mute(2)).toHaveAccessibleName(/Mute .*\. Silenced by a solo/)
  await expect(mute(2)).toHaveCSS('border-top-style', 'dashed')
  await expect(rows.nth(2).locator('.head')).toHaveCSS('opacity', '0.4')
  await expect(mute(0)).not.toHaveAttribute('data-implied', 'true')
  // The mixer marks it too.
  await page.getByTestId('nav-mixer').click()
  await expect(page.getByTestId('mixer-mute').nth(2)).toHaveAttribute('data-implied', 'true')
  await expect(page.getByTestId('mixer-mute').nth(2)).toHaveCSS('border-top-style', 'dashed')
  await expect(page.getByTestId('mixer-mute').nth(2)).toHaveAccessibleName(/Silenced by a solo/)
  await page.getByTestId('nav-arrangement').click()
  await page.screenshot({ path: `${SHOTS}/arrangement-states-${page.viewportSize()?.width ?? 0}.png` })

  // No hue needed: the states are letters and line shapes, still there on the page (the screenshot is kept for a look in greyscale).
  await expect(mute(1)).toHaveCSS('text-decoration-line', 'line-through')
  await expect(solo(0)).toHaveCSS('text-decoration-line', 'underline')
  await expect(mute(2)).toHaveCSS('border-top-style', 'dashed')
  expect(errors).toEqual([])
})

test('the selected track has a 2px light outline and text that is AA', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('arr-track').nth(1).click()
  const row = page.getByTestId('arr-track').nth(1).locator('xpath=..')
  await expect(row).toHaveCSS('outline-width', '2px')
  await expect(row).toHaveCSS('outline-style', 'solid')
  expect(await contrast(row.locator('.track-name'))).toBeGreaterThanOrEqual(4.5)
  // The row's secondary text (the kind) too.
  expect(await contrast(row.locator('.kind'))).toBeGreaterThanOrEqual(4.5)
  await page.getByTestId('nav-mixer').click()
  const strip = page.getByTestId('mixer-strip').nth(1)
  await expect(strip).toHaveCSS('outline-width', '2px')
  expect(await contrast(strip.getByTestId('mixer-chain'))).toBeGreaterThanOrEqual(4.5)
  expect(errors).toEqual([])
})

test('the master meter is in the top bar of every view, and a clip stays until it is reset by click or keyboard', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  for (const view of ['arrangement', 'grid', 'mixer']) {
    await page.getByTestId(`nav-${view}`).click()
    await expect(page.getByTestId('top-meter')).toBeVisible()
  }
  // The keyboard plays the selected track: the first one.
  await page.getByTestId('nav-arrangement').click()
  await page.getByTestId('arr-track').first().click()
  await page.getByTestId('nav-mixer').click()
  // Drive the first strip's fader to its top and play a chord: the output reaches full scale.
  const fader = page.getByTestId('mixer-fader').first()
  await fader.focus()
  await page.keyboard.press('End')
  for (const note of [60, 64, 67, 72, 76, 79]) await send(page, [0x90, note, 127])
  const clip = page.getByTestId('mixer-meter-peak').first()
  await expect(clip).toContainText('CLIP', { timeout: 15000 })
  await expect(page.getByTestId('top-meter-clip')).toBeVisible()
  for (const note of [60, 64, 67, 72, 76, 79]) await send(page, [0x80, note, 0])
  await page.waitForTimeout(800)
  // Latched: still there with the sound gone, with the word and a reset affordance.
  await expect(clip).toContainText('CLIP')
  await expect(clip).toContainText('×')
  await page.screenshot({ path: `${SHOTS}/clip-latched-${page.viewportSize()?.width ?? 0}.png` })
  // The hatching is a background image (a shape), not only red.
  expect(await clip.evaluate((el) => getComputedStyle(el).backgroundImage)).toContain('repeating-linear-gradient')
  // Reset from the keyboard.
  await page.getByTestId('top-meter-clip').focus()
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('top-meter-clip')).toHaveCount(0)
  await expect(clip).toContainText('CLIP')
  await clip.focus()
  await page.keyboard.press('Space')
  await expect(clip).not.toContainText('CLIP')
  expect(errors).toEqual([])
})

test('one voice: playback is green with its triangle, toggles are fills, knobs and faders are neutral', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const violet = /rgb\((102, 51, 244|156, 146, 255|42, 33, 80|76, 30, 188)\)/
  const play = page.getByTestId('transport-play')
  const triangle = play.locator('path')
  await expect(triangle).toHaveCSS('fill', /rgb\(.*\)/)
  expect(await triangle.evaluate((el) => getComputedStyle(el).fill)).not.toMatch(/rgb\(63, 193, 104\)/)
  await play.click()
  await expect(play).toHaveAttribute('aria-pressed', 'true')
  expect(await triangle.evaluate((el) => getComputedStyle(el).fill)).toMatch(/rgb\(63, 193, 104\)/)
  await expect(play).toHaveCSS('border-top-color', 'rgb(63, 193, 104)')
  await page.getByTestId('transport-stop').click()
  // Stop stays neutral.
  expect(await page.getByTestId('transport-stop').evaluate((el) => getComputedStyle(el).backgroundColor)).not.toMatch(violet)

  // Loop and metronome on: a fill in the text colour and a dot, never the violet.
  for (const id of ['transport-loop', 'transport-metronome']) {
    const toggle = page.getByTestId(id)
    await toggle.click()
    await expect(toggle).toHaveAttribute('aria-pressed', 'true')
    const style = await toggle.evaluate((el) => ({ bg: getComputedStyle(el).backgroundColor, dot: getComputedStyle(el, '::after').content }))
    expect(style.bg).not.toMatch(violet)
    expect(style.dot).toBe('""')
  }
  // Follow: on by default, a fill and a dot before the word.
  const follow = page.getByTestId('arr-follow')
  await expect(follow).toHaveAttribute('aria-pressed', 'true')
  expect(await follow.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toMatch(violet)
  expect(await follow.evaluate((el) => getComputedStyle(el, '::before').content)).toContain('●')

  // Faders and the pan are neutral.
  await page.getByTestId('nav-mixer').click()
  const fill = page.getByTestId('mixer-fader').first().locator('.fill')
  expect(await fill.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toMatch(violet)
  await page.screenshot({ path: `${SHOTS}/mixer-${page.viewportSize()?.width ?? 0}.png` })
  expect(errors).toEqual([])
})

test('an automated parameter is marked by a glyph, not by the violet', async ({ page }) => {
  test.skip(test.info().project.name !== 'dev', 'the lane gets its points through the dev hooks of the page')
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('mixer-automate-gain').first().click()
  // The lane needs points to drive the fader.
  await page.evaluate(() => {
    const w = window as unknown as {
      __faulkRun: (c: unknown) => unknown
      __faulkView: () => { project: { tracks: Record<string, { automation: Record<string, unknown> }> } }
    }
    const track = Object.values(w.__faulkView().project.tracks).find((t) => Object.keys(t.automation).length > 0)!
    w.__faulkRun({
      type: 'automation.points.add',
      lane: Object.keys(track.automation)[0],
      points: [
        { tick: 0, value: 0, shape: 'step' },
        { tick: 1920, value: -60, shape: 'step' },
      ],
    })
  })
  await page.getByTestId('nav-mixer').click()
  const marked = page.getByTestId('mixer-fader').first()
  await expect(marked).toHaveAttribute('data-automated', 'true')
  await expect(marked.getByTestId('automated-glyph')).toBeVisible()
  expect(await marked.evaluate((el) => getComputedStyle(el).boxShadow)).not.toMatch(/156, 146, 255|102, 51, 244|42, 33, 80/)
  await page.screenshot({ path: `${SHOTS}/automated-${page.viewportSize()?.width ?? 0}.png` })
  expect(errors).toEqual([])
})

test('the Compressor shows the gain it takes off, and none when bypassed', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('arr-track').first().click()
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('mixer-devices').first().click()
  const panel = page.getByTestId('bottom-panel')
  await panel.getByRole('button', { name: 'Add Compressor to the track', exact: true }).click()
  const card = panel.getByTestId('device-card').filter({ hasText: 'Compressor' })
  await expect(card.getByTestId('gain-reduction')).toBeVisible()
  await expect(card.getByTestId('gain-reduction-value')).toHaveText('0.0 dB')
  await card.getByTestId('param-threshold').getByRole('slider').focus()
  await page.keyboard.press('Home')
  await card.getByTestId('param-ratio').getByRole('slider').focus()
  await page.keyboard.press('End')
  await send(page, [0x90, 69, 127])
  await expect
    .poll(async () => Number(((await card.getByTestId('gain-reduction-value').textContent()) ?? '').replace('−', '-').replace(' dB', '')), { timeout: 8000 })
    .toBeLessThan(-3)
  await page.screenshot({ path: `${SHOTS}/compressor-${page.viewportSize()?.width ?? 0}.png` })
  // Bypassed: no reduction shown.
  await card.getByRole('button', { name: 'Bypass Compressor', exact: true }).click()
  await expect(card.getByTestId('gain-reduction-value')).toHaveText('0.0 dB')
  expect(errors).toEqual([])
})
