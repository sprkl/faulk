import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// DV-9 (#70): the device panel in a real browser. Real worklet, engine and project core: every
// action goes through the dispatcher as a DV-8 command. The registry has no effect yet (DV-3 to
// DV-5), so the effect flow (insert, move, remove) is covered by unit tests on the commands the
// panel sends; here the effects section shows its empty state.

const TEXTS = {
  en: { bypass: (n: string) => `Bypass ${n}`, use: (n: string) => `Use ${n} as the instrument`, noEffects: 'No effect yet.', hint: 'Output level of the synth, from 0 to 1 (linear gain).' },
  fr: { bypass: (n: string) => `Contourner ${n}`, use: (n: string) => `Utiliser ${n} comme instrument`, noEffects: 'Aucun effet pour l’instant.', hint: 'Niveau de sortie du synthé, de 0 à 1 (gain linéaire).' },
} as const

const card = (page: Page, name: string) => page.getByTestId('device-card').filter({ has: page.getByRole('heading', { name, exact: true }) })

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

for (const lang of ['en', 'fr'] as const) {
  test(`devices, ${lang}: the panel is generated for Oscillator and Drums, switches instrument, bypasses and undoes`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await expect(page.getByTestId('devices-empty')).toBeVisible()
    await page.getByTestId('engine-state').getByRole('button').click()
    await expect(page.getByTestId('engine-mode')).toBeVisible()

    // The Oscillator track: one card, a control per parameter of its descriptor.
    const panel = page.getByTestId('bottom-panel')
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    const osc = card(page, 'Oscillator')
    await expect(osc.locator('[data-testid^=param-]')).toHaveCount(6)
    await expect(osc.getByTestId('device-remove')).toHaveCount(0)
    await expect(osc.getByTestId('device-left')).toHaveCount(0)
    await expect(panel.getByTestId('browse-no-effects')).toHaveText(x.noEffects)
    // The tooltip of a parameter is its description, in the page's language.
    await expect(osc.getByTestId('param-gain').locator('label')).toHaveAttribute('title', x.hint)

    // A choice and a knob reach the project.
    await osc.getByRole('radio', { name: lang === 'en' ? 'Square' : 'Carré' }).click()
    await expect(osc.getByRole('radio', { name: lang === 'en' ? 'Square' : 'Carré' })).toHaveAttribute('aria-checked', 'true')
    const attack = osc.getByTestId('param-attack').locator('input')
    const before = await attack.getAttribute('aria-valuetext')

    // A drag of a knob is one history entry: one undo takes it back whole.
    const box = (await attack.boundingBox())!
    await page.mouse.move(box.x + box.width * 0.1, box.y + box.height / 2)
    await page.mouse.down()
    await page.mouse.move(box.x + box.width * 0.5, box.y + box.height / 2, { steps: 6 })
    await page.mouse.move(box.x + box.width * 0.9, box.y + box.height / 2, { steps: 6 })
    await page.mouse.up()
    await expect.poll(() => attack.getAttribute('aria-valuetext')).not.toBe(before)
    await page.keyboard.press('Control+z')
    await expect.poll(() => attack.getAttribute('aria-valuetext')).toBe(before)
    await expect(osc.getByRole('radio', { name: lang === 'en' ? 'Square' : 'Carré' })).toHaveAttribute('aria-checked', 'true')

    // Bypass the instrument, then undo.
    const bypass = osc.getByRole('button', { name: x.bypass('Oscillator'), exact: true })
    await bypass.click()
    await expect(bypass).toHaveAttribute('aria-pressed', 'true')
    await expect(osc).toHaveAttribute('data-bypassed', 'true')
    await page.keyboard.press('Control+z')
    await expect(bypass).toHaveAttribute('aria-pressed', 'false')

    // Switch the instrument from the browser: the panel is Drums' own, generated the same way, then back.
    await panel.getByRole('button', { name: x.use('Drums'), exact: true }).click()
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    const drums = card(page, 'Drums')
    await expect(drums.locator('[data-testid^=param-]')).toHaveCount(16)
    await expect(panel.getByRole('button', { name: x.use('Drums'), exact: true })).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.press('Control+z')
    await expect(card(page, 'Oscillator')).toHaveCount(1)

    // A Drums track of its own: its panel on selection, a knob changes.
    await addTrack(page, 'Drums')
    const kit = card(page, 'Drums')
    await expect(kit.locator('[data-testid^=param-]')).toHaveCount(16)
    const tune = kit.getByTestId('param-kick_tune').locator('input')
    const tuneBefore = await tune.getAttribute('aria-valuetext')
    await tune.fill('0.9')
    await expect.poll(() => tune.getAttribute('aria-valuetext')).not.toBe(tuneBefore)

    // Nothing sticks out sideways.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)
    expect(overflow).toBe(false)
    expect(errors).toEqual([])
  })
}
