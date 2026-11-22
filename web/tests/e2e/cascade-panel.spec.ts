import { expect, test, type Page } from '@playwright/test'
import { DEVICES } from '../../src/lib/generated/protocol'
import { engineUp, watch } from './support'

// #166: Cascade, the arpeggiator MIDI effect, in a real browser. The device browser puts it before the instrument of the
// track; its generated panel shows every parameter of its descriptor with its label in the page's language (the device's
// name is the same English one in both); no console error.

const LABELS = {
  en: {
    order: 'Order',
    rate: 'Rate',
    feel: 'Feel',
    octaves: 'Octaves',
    repeats: 'Repeats',
    gate: 'Gate',
    velocity_source: 'Velocity from',
    latch: 'Latch',
    restart: 'Restart',
    seed: 'Seed',
  },
  fr: {
    order: 'Ordre',
    rate: 'Division',
    feel: 'Type',
    octaves: 'Octaves',
    repeats: 'Répétitions',
    gate: 'Durée',
    velocity_source: 'Vélocité',
    latch: 'Maintien',
    restart: 'Redémarrage',
    seed: 'Graine',
  },
} as const

async function start(page: Page, lang: 'en' | 'fr') {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

for (const lang of ['en', 'fr'] as const) {
  test(`cascade panel, ${lang}: inserted before the instrument, every parameter shown with its label`, async ({ page }) => {
    const { errors } = watch(page)
    await start(page, lang)
    await page.keyboard.press('Alt+KeyB')
    const search = page.getByTestId('browser-search')
    await expect(search).toBeFocused()
    await search.fill('cascade')
    await expect(page.getByTestId('browse-device')).toHaveText(['Cascade'])
    await page.keyboard.press('Enter')
    const kinds = await page.getByTestId('device-card').evaluateAll((cards) => cards.map((c) => c.getAttribute('data-kind')))
    expect(kinds).toEqual(['faulk.cascade', 'faulk.osc_synth'])
    const card = page.locator('[data-testid="device-card"][data-kind="faulk.cascade"]')
    await expect(card).toContainText('Cascade')
    const descriptor = DEVICES.find((d) => d.kind === 'faulk.cascade')!
    expect(descriptor.params).toHaveLength(12)
    for (const p of descriptor.params) await expect(card.getByTestId(`param-${p.key}`)).toBeVisible()
    for (const label of Object.values(LABELS[lang])) await expect(card.getByLabel(label, { exact: true }).first()).toBeVisible()
    expect(errors).toEqual([])
  })
}
