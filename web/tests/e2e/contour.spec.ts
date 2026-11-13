import { expect, test, type Page } from '@playwright/test'
import { fileURLToPath } from 'node:url'
import { chooseMidiInput, engineUp, fakeWebMidi, watch } from './support'

// #156: Contour's panel in a real browser. Real worklet, engine and project core: the spectrum of
// its output comes from the engine's report (computed off the audio path), the curve follows its
// settings, and a drag of a handle is one history group of `device.set_param`. With
// FAULK_E2E_SHOTS set, screenshots of the panel in the dark and the light theme go to `.qa/156/` (or to `.qa/<value>/` when the value is not 1).

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
    await handle.scrollIntoViewIfNeeded()
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

for (const lang of ['en', 'fr'] as const) {
  test(`contour, ${lang}: a click on a point focuses it and the arrows set it, the controls are grouped by band, the help is on a button`, async ({ page }) => {
    const { errors } = watch(page)
    const card = await contourWithANote(page, lang)
    const curve = card.getByTestId('contour-curve')
    await expect(curve.getByTestId('contour-handle')).toHaveCount(7)

    // The controls: seven bands, each with a visible legend and its own switch, then the output.
    const bands = card.getByTestId('contour-band')
    await expect(bands).toHaveCount(8)
    const legends = card.getByTestId('contour-band-legend')
    for (let i = 0; i < 7; i++) await expect(legends.nth(i)).toBeVisible()
    const expected =
      lang === 'en'
        ? ['Low cut', 'Low shelf', 'Bell 1', 'Bell 2', 'Bell 3', 'High shelf', 'High cut']
        : ['Coupe-bas', 'Plateau grave', 'Cloche 1', 'Cloche 2', 'Cloche 3', 'Plateau aigu', 'Coupe-haut']
    await expect(legends).toHaveText(expected.map((text, i) => new RegExp(`${i + 1}\\s*${text}`)))
    await expect(bands.nth(3).getByTestId('param-bell2_on')).toBeVisible()
    await expect(bands.nth(3).getByTestId('param-bell2_gain')).toBeVisible()
    await expect(bands.nth(3).getByTestId('param-bell3_gain')).toHaveCount(0)
    await expect(bands.nth(3)).toHaveAccessibleName(new RegExp(expected[3]))

    // A click on a point (no move) gives it the focus; the arrows then set its value at once.
    const handle = curve.locator('[data-testid="contour-handle"][data-band="bell2"]')
    await handle.scrollIntoViewIfNeeded()
    await handle.click()
    await expect(handle).toBeFocused()
    const gain = card.getByTestId('param-bell2_gain').getByRole('slider')
    await expect(gain).toHaveAttribute('aria-valuetext', '0.0 dB')
    await page.keyboard.press('ArrowUp')
    await page.keyboard.press('ArrowUp')
    await expect(gain).toHaveAttribute('aria-valuetext', '1.0 dB')
    await page.keyboard.press('ArrowDown')
    await expect(gain).toHaveAttribute('aria-valuetext', '0.5 dB')
    // Shift with the arrows sets the Q.
    const qBefore = await card.getByTestId('param-bell2_q').getByRole('slider').getAttribute('aria-valuetext')
    await page.keyboard.press('Shift+ArrowUp')
    await expect(card.getByTestId('param-bell2_q').getByRole('slider')).not.toHaveAttribute('aria-valuetext', qBefore!)
    // A click on another point moves the focus there.
    const other = curve.locator('[data-testid="contour-handle"][data-band="high"]')
    await other.click()
    await expect(other).toBeFocused()

    // The help: a button, by the keyboard, shows the text; a screen reader gets it as each point's description.
    const toggle = card.getByTestId('contour-help-toggle')
    const help = card.getByTestId('contour-help')
    await expect(toggle).toHaveAttribute('aria-expanded', 'false')
    await expect(help).toHaveClass(/sr-only/)
    await toggle.focus()
    await page.keyboard.press('Enter')
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')
    await expect(help).not.toHaveClass(/sr-only/)
    await expect(help).toContainText(lang === 'en' ? 'Drag a point' : 'Fais glisser un point')
    await expect(handle).toHaveAccessibleDescription(await help.innerText())
    await page.keyboard.press('Enter')
    await expect(help).toHaveClass(/sr-only/)

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
  const dir = fileURLToPath(new URL(`../../../.qa/${SHOTS === '1' ? '156' : SHOTS}/`, import.meta.url))
  // The bottom panel maximised: the whole card in view.
  await page.getByTestId('bottom-panel-maximize').click()
  // Tall enough for the whole card, whichever way its controls are laid out.
  await page.setViewportSize({ width: 1440, height: 1500 })
  const shoot = async (theme: string) => {
    await page.waitForTimeout(800)
    await card.scrollIntoViewIfNeeded()
    await card.screenshot({ path: `${dir}contour-${theme}.png` })
    await curve.screenshot({ path: `${dir}contour-curve-${theme}.png` })
  }
  await shoot('dark')
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-theme-light').click()
  await page.keyboard.press('Escape')
  await shoot('light')
})
