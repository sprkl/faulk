import { expect, test, type Page } from '@playwright/test'
import { chooseMidiInput, fakeWebMidi, watch } from './support'

// RE-1 (#85): the live MIDI input goes to the selected track and to the armed ones, and a note held
// while the selection or the arming changes still ends. Real worklet, engine and project core; the
// per-track peaks of the hidden `feedback` element say which track sounds.

const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

const peaks = async (page: Page) => ((await page.getByTestId('feedback').getAttribute('data-track-peaks')) ?? '').split(',').map(Number)

const masterPeak = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))

const header = (page: Page, name: string) => page.getByTestId('track-header').filter({ has: page.getByRole('button', { name, exact: true }) })

const TEXTS = {
  en: { arm: (name: string) => `Arm ${name} to record`, drums: 'Track 2' },
  fr: { arm: (name: string) => `Armer ${name} pour l’enregistrer`, drums: 'Piste 2' },
} as const

for (const lang of ['en', 'fr'] as const) {
  test(`live input, ${lang}: selected and armed tracks play, a held note ends after a track change`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await fakeWebMidi(page)
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button').click()
    await expect(page.getByTestId('engine-mode')).toBeVisible()
    await chooseMidiInput(page, 'E2E Keys')
    // The tracks are selected and armed on their rows in the arrangement (#471).
    await page.getByTestId('nav-arrangement').click()
    await expect(header(page, 'Synth')).toHaveAttribute('aria-current', 'true')

    // A second track, selected on creation: the first one is neither selected nor armed.
    await page.getByTestId('add-track').click()
    await page.getByRole('menuitem', { name: 'Oscillator', exact: true }).click()
    await expect(header(page, x.drums)).toHaveAttribute('aria-current', 'true')
    await send(page, [0x90, 57, 127])
    await expect.poll(async () => (await peaks(page))[1]).toBeGreaterThan(0.05)
    expect((await peaks(page))[0]).toBeLessThan(0.001)
    await send(page, [0x80, 57, 0])
    await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)

    // Armed, the first track plays as well, without being selected.
    const arm = header(page, 'Synth').getByRole('button', { name: x.arm('Synth'), exact: true })
    await arm.click()
    await expect(arm).toHaveAttribute('aria-pressed', 'true')
    await expect(header(page, 'Synth')).not.toHaveAttribute('aria-current', 'true')
    await send(page, [0x90, 57, 127])
    await expect.poll(async () => Math.min(...(await peaks(page)).slice(0, 2))).toBeGreaterThan(0.05)

    // Held: the selection moves to the first track and the second is unarmed (never was): the note
    // off still reaches both tracks that started it.
    await header(page, 'Synth').getByTestId('track-name').click()
    await expect(header(page, 'Synth')).toHaveAttribute('aria-current', 'true')
    await arm.click()
    await expect(arm).toHaveAttribute('aria-pressed', 'false')
    await send(page, [0x80, 57, 0])
    await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)

    // A controller message changes nothing audible and raises no error.
    await send(page, [0xb0, 1, 100])
    expect(errors).toEqual([])
  })
}
