import { expect, test, type Page } from '@playwright/test'
import { chooseMidiInput, fakeWebMidi, watch } from './support'

// #364: the master's chain of effects in a real browser. Real worklet, engine and project core:
// the Master strip of the mixer shows "Devices (n)", the bottom panel edits the master's chain
// like a track's (effects only), and the sound goes through it (the master meter reads after it).

const TEXTS = {
  en: { devices: 'Devices', add: (n: string) => `Add ${n} to the master`, bypass: (n: string) => `Bypass ${n}`, remove: (n: string) => `Remove ${n}` },
  fr: { devices: 'Devices', add: (n: string) => `Ajouter ${n} au master`, bypass: (n: string) => `Contourner ${n}`, remove: (n: string) => `Retirer ${n}` },
} as const

const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

for (const lang of ['en', 'fr'] as const) {
  test(`master chain, ${lang}: the Master strip lists the devices, the panel edits them, the sound goes through`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await fakeWebMidi(page)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button').click()
    await expect(page.getByTestId('engine-mode')).toBeVisible()
    await chooseMidiInput(page, 'E2E Keys')
    await page.getByTestId('nav-mixer').click()
    const master = page.getByTestId('mixer-master')
    const button = master.getByTestId('mixer-master-devices')
    await expect(button).toHaveText(`${x.devices} (0)`)

    // A held note: the master meter, with no chain.
    await send(page, [0x90, 69, 127])
    const peak = async () => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
    await expect.poll(peak).toBeGreaterThan(0.05)
    await page.waitForTimeout(400)
    const dry = await peak()

    // The master's panel: effects only.
    await button.click()
    const panel = page.getByTestId('bottom-panel')
    await expect(panel).toHaveAttribute('data-panel', 'devices')
    await expect(page.getByTestId('devices')).toHaveAttribute('data-target', 'master')
    await expect(panel.getByTestId('browse-instrument')).toHaveCount(0)
    await expect(panel.getByTestId('device-card')).toHaveCount(0)

    // A compressor on the master, pulled down hard: the meter (after the chain) drops.
    await panel.getByRole('button', { name: x.add('Compressor'), exact: true }).click()
    await expect(panel.getByTestId('device-card')).toHaveCount(1)
    await expect(button).toHaveText(`${x.devices} (1)`)
    const comp = panel.getByTestId('device-card').first()
    const threshold = comp.getByTestId('param-threshold').getByRole('slider')
    await threshold.focus()
    await page.keyboard.press('Home')
    const ratio = comp.getByTestId('param-ratio').getByRole('slider')
    await ratio.focus()
    await page.keyboard.press('End')
    await expect.poll(async () => (await peak()) / dry, { timeout: 8000 }).toBeLessThan(0.8)

    // Bypass: the level comes back. Undo of the bypass: the chain acts again.
    const bypass = comp.getByRole('button', { name: x.bypass('Compressor'), exact: true })
    await bypass.click()
    await expect(bypass).toHaveAttribute('aria-pressed', 'true')
    await expect.poll(async () => (await peak()) / dry, { timeout: 8000 }).toBeGreaterThan(0.9)
    await page.keyboard.press('Control+z')
    await expect(bypass).toHaveAttribute('aria-pressed', 'false')

    // Remove, then undo brings it back with its settings.
    await comp.getByRole('button', { name: x.remove('Compressor'), exact: true }).click()
    await expect(panel.getByTestId('device-card')).toHaveCount(0)
    await expect(button).toHaveText(`${x.devices} (0)`)
    await page.keyboard.press('Control+z')
    await expect(panel.getByTestId('device-card')).toHaveCount(1)
    await expect(button).toHaveText(`${x.devices} (1)`)

    // Selecting a track takes the panel back to the track's chain (an instrument and a browser of instruments).
    await page.getByTestId('mixer-strip').first().getByTestId('mixer-name').click()
    await expect(page.getByTestId('devices')).not.toHaveAttribute('data-target', 'master')

    await send(page, [0x80, 69, 0])
    expect(errors).toEqual([])
  })
}
