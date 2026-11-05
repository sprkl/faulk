import { expect, test, type Page } from '@playwright/test'
import { fileURLToPath } from 'node:url'
import { chooseMidiInput, engineUp, fakeWebMidi, watch } from './support'

// #156: Contour's panel in a real browser. Real worklet, engine and project core: the spectrum of
// its output comes from the engine's report (computed off the audio path), the curve follows its
// settings, and a drag of a handle is one history group of `device.set_param`. With
// FAULK_E2E_SHOTS set, screenshots of the panel in the dark and the light theme go to `.qa/156/`.

test.describe.configure({ mode: 'serial', timeout: 120_000 })

const SHOTS = process.env.FAULK_E2E_SHOTS
const TEXTS = {
  en: { add: 'Add Contour to the track', curve: 'Response curve of Contour', off: 'off' },
  fr: { add: 'Ajouter Contour à la piste', curve: 'Courbe de réponse de Contour', off: 'coupée' },
} as const

const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

async function contourWithANote(page: Page, lang: 'en' | 'fr') {
  await fakeWebMidi(page)
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page, 'Web')
  await chooseMidiInput(page, 'E2E Keys')
  await page.getByTestId('bottom-panel').getByRole('button', { name: TEXTS[lang].add, exact: true }).click()
  const card = page.getByTestId('device-card').filter({ has: page.getByRole('heading', { name: 'Contour', exact: true }) })
  await expect(card).toHaveCount(1)
  // A held chord: the spectrum has something to show.
  for (const key of [45, 57, 64]) await send(page, [0x90, key, 110])
  return card
}

for (const lang of ['en', 'fr'] as const) {
  test(`contour, ${lang}: the spectrum shows, a drag is one undo, a double click switches a band`, async ({ page }) => {
    const { errors } = watch(page)
    const card = await contourWithANote(page, lang)
    const curve = card.getByTestId('contour-curve')
    await expect(curve.getByRole('group', { name: TEXTS[lang].curve })).toBeVisible()
    await expect(curve.getByTestId('contour-handle')).toHaveCount(7)
    // The engine's spectrum of Contour's output reaches the panel.
    await expect(curve).toHaveAttribute('data-spectrum', 'on', { timeout: 10_000 })
    await expect(curve.getByTestId('contour-spectrum')).toHaveAttribute('d', /L/)

    // Drag bell 2 up and right: its gain and its frequency change, and one undo takes both back.
    const gain = card.getByTestId('param-bell2_gain').getByRole('slider')
    const freq = card.getByTestId('param-bell2_freq').getByRole('slider')
    await expect(gain).toHaveAttribute('aria-valuetext', '0.0 dB')
    const freqBefore = await freq.getAttribute('aria-valuetext')
    const curveBefore = await curve.getByTestId('contour-response').getAttribute('d')
    const handle = curve.locator('[data-testid="contour-handle"][data-band="bell2"]')
    const box = (await handle.boundingBox())!
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.down()
    for (let i = 1; i <= 8; i++) await page.mouse.move(box.x + box.width / 2 + 5 * i, box.y + box.height / 2 - 4 * i)
    await page.mouse.up()
    await expect(gain).not.toHaveAttribute('aria-valuetext', '0.0 dB')
    await expect(freq).not.toHaveAttribute('aria-valuetext', freqBefore!)
    await expect(curve.getByTestId('contour-response')).not.toHaveAttribute('d', curveBefore!)
    await page.keyboard.press('Control+z')
    await expect(gain).toHaveAttribute('aria-valuetext', '0.0 dB')
    await expect(freq).toHaveAttribute('aria-valuetext', freqBefore!)

    // A double click switches the low cut on (its switch follows), one undo switches it back.
    const lowCut = curve.locator('[data-testid="contour-handle"][data-band="hp"]')
    await expect(lowCut).toHaveAttribute('aria-label', new RegExp(`, ${TEXTS[lang].off}$`))
    await lowCut.dblclick()
    await expect(lowCut).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.press('Control+z')
    await expect(lowCut).toHaveAttribute('aria-pressed', 'false')

    // Keyboard: a focused handle moves with the arrows.
    await curve.locator('[data-testid="contour-handle"][data-band="bell3"]').focus()
    await page.keyboard.press('ArrowUp')
    await expect(card.getByTestId('param-bell3_gain').getByRole('slider')).toHaveAttribute('aria-valuetext', '0.5 dB')

    // Nothing sticks out sideways.
    expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false)
    for (const key of [45, 57, 64]) await send(page, [0x80, key, 0])
    expect(errors).toEqual([])
  })
}

test('contour: screenshots of the panel in the dark and the light theme', async ({ page }) => {
  test.skip(!SHOTS, 'FAULK_E2E_SHOTS is not set')
  const card = await contourWithANote(page, 'en')
  const curve = card.getByTestId('contour-curve')
  await expect(curve).toHaveAttribute('data-spectrum', 'on', { timeout: 10_000 })
  // A shape worth seeing, by the handles' keys: a low cut at 60 Hz, a cut near 250 Hz, a lift
  // near 3 kHz, a little air.
  const press = async (band: string, key: string, times: number) => {
    await curve.locator(`[data-testid="contour-handle"][data-band="${band}"]`).focus()
    for (let i = 0; i < times; i++) await page.keyboard.press(key)
  }
  await press('hp', 'Enter', 1)
  await press('hp', 'ArrowRight', 6)
  await press('bell1', 'ArrowDown', 12)
  await press('bell3', 'ArrowLeft', 2)
  await press('bell3', 'ArrowUp', 12)
  await press('high', 'ArrowUp', 6)
  const dir = fileURLToPath(new URL('../../../.qa/156/', import.meta.url))
  await page.waitForTimeout(800)
  await card.screenshot({ path: `${dir}contour-dark.png` })
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-theme-light').click()
  await page.keyboard.press('Escape')
  await page.waitForTimeout(800)
  await card.screenshot({ path: `${dir}contour-light.png` })
})
