import { expect, test, type Page } from '@playwright/test'
import { chooseMidiInput, engineUp, fakeWebMidi, watch } from './support'

// #331: a double-click on Stop cuts all sound at once. A note played into an Echo at its highest
// feedback keeps ringing after the note ends and after Stop; the double-click brings the master
// level to zero and it stays there. A single click on Stop keeps its meaning (the transport stops,
// the tail goes on). The hidden `feedback` element carries the engine's report.

test.describe.configure({ mode: 'serial' })

const feedback = (page: Page) => page.getByTestId('feedback')
const master = async (page: Page) => Number(await feedback(page).getAttribute('data-master-peak'))
const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

const TEXTS = {
  en: { add: 'Add Echo to the track', hint: 'Stop and go back to the marker. Stop again: back to the start. Double-click to also cut all sound.' },
  fr: { add: 'Ajouter Echo à la piste', hint: 'Stop et retour au repère. Stop encore : retour au début. Double-clic pour couper aussi tout le son.' },
} as const

for (const lang of ['en', 'fr'] as const) {
  test(`all sound off, ${lang}: a double-click on Stop silences an echo that rings on`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await fakeWebMidi(page)
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button').click()
    await engineUp(page)
    await chooseMidiInput(page, 'E2E Keys')
    await expect(page.getByTestId('track-list')).toHaveAttribute('data-track-count', '1')

    // An Echo after the synth, at its highest feedback and fully wet.
    await page.getByTestId('bottom-panel').getByRole('button', { name: x.add, exact: true }).click()
    const echo = page.getByTestId('device-card').filter({ has: page.getByRole('heading', { name: 'Echo', exact: true }) })
    await expect(echo).toHaveCount(1)
    await echo.getByTestId('param-feedback').getByRole('slider').press('End')
    await echo.getByTestId('param-mix').getByRole('slider').press('End')

    const stop = page.getByTestId('transport-stop')
    await expect(stop).toHaveAttribute('title', x.hint)

    // A note, then its release: the echo rings on.
    await send(page, [0x90, 69, 127])
    await expect.poll(() => master(page)).toBeGreaterThan(0.05)
    await page.waitForTimeout(600)
    await send(page, [0x80, 69, 0])
    await page.waitForTimeout(1500)
    expect(await master(page)).toBeGreaterThan(0.01)

    // A single click on Stop leaves the tail alone.
    await stop.click()
    await page.waitForTimeout(500)
    expect(await master(page)).toBeGreaterThan(0.005)

    // The double-click cuts everything, and the level stays at zero.
    await stop.dblclick()
    await expect.poll(() => master(page), { timeout: 5000 }).toBe(0)
    await page.waitForTimeout(1500)
    expect(await master(page)).toBe(0)
    expect(errors).toEqual([])
  })
}
