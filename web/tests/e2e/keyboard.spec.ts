import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// UI-3 (#107): the computer keyboard plays through the live input commands, so it reaches the
// selected MIDI track and the armed ones; a track change while a key is held ends the note where it
// started; the drums start at the octave where their sounds are; no shortcut fires while playing.
// Real worklet, engine and project core; the per-track peaks of the hidden `feedback` element say
// which track sounds.

const peaks = async (page: Page) => ((await page.getByTestId('feedback').getAttribute('data-track-peaks')) ?? '').split(',').map(Number)
/**
 * The highest per-track peak seen since `startPeakHold`. The meter is read at 30 Hz and a short sound (a snare) can
 * fall between two readings the test polls, so the page keeps the crest itself, on every update of the element.
 */
const startPeakHold = (page: Page) =>
  page.evaluate(() => {
    const el = document.querySelector('[data-testid=feedback]')!
    const w = window as unknown as { __heldPeaks?: number[] }
    // Called again, it starts a new hold: the crest of the previous key must not stand for the next.
    const held = (w.__heldPeaks ??= [])
    held.fill(0)
    const read = () => (el.getAttribute('data-track-peaks') ?? '').split(',').forEach((v, i) => (held[i] = Math.max(held[i] ?? 0, Number(v) || 0)))
    if (!(w as { __holdObserver?: boolean }).__holdObserver) {
      new MutationObserver(read).observe(el, { attributes: true, attributeFilter: ['data-track-peaks'] })
      ;(w as { __holdObserver?: boolean }).__holdObserver = true
    }
    read()
  })
const heldPeak = (page: Page, track: number) => page.evaluate((i) => (window as unknown as { __heldPeaks: number[] }).__heldPeaks[i] ?? 0, track)
const masterPeak = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
const header = (page: Page, name: string) => page.getByTestId('track-header').filter({ has: page.getByRole('button', { name, exact: true }) })
const selectedHeader = (page: Page) => page.locator('[data-testid=track-header][aria-current=true]')

const TEXTS = {
  en: {
    mute: (name: string) => `Mute ${name}`,
    arm: (name: string) => `Arm ${name} to record`,
    octave: (n: number) => `Octave ${n}`,
    target: (name: string) => `Playing ${name}.`,
    armed: 'and the armed tracks.',
  },
  fr: {
    mute: (name: string) => `Couper ${name}`,
    arm: (name: string) => `Armer ${name} pour l’enregistrer`,
    octave: (n: number) => `Octave ${n}`,
    target: (name: string) => `Tu joues sur ${name}.`,
    armed: 'et sur les pistes armées.',
  },
} as const

for (const lang of ['en', 'fr'] as const) {
  test(`computer keyboard, ${lang}: oscillator and drums, track switch while held, armed track, no shortcut`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button').click()
    await expect(page.getByTestId('engine-mode')).toBeVisible()
    await expect(page.getByTestId('inspector-name')).toHaveText('Synth')
    await page.getByTestId('panel-keyboard').click()
    await expect(page.getByTestId('keys-target')).toHaveText(x.target('Synth'))
    await expect(page.getByText(x.octave(4), { exact: true })).toBeVisible()
    // The tracks are selected, armed and muted on their rows in the arrangement (#471); the keyboard plays in every view.
    await page.getByTestId('nav-arrangement').click()
    await expect(header(page, 'Synth')).toHaveAttribute('aria-current', 'true')

    // The oscillator track sounds at octave 4 (A is C4).
    await page.keyboard.down('KeyA')
    await expect.poll(async () => (await peaks(page))[0]).toBeGreaterThan(0.05)
    await page.keyboard.up('KeyA')
    await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)

    // A drums track, selected on creation: the keyboard starts at octave 2, A is the kick.
    await page.getByTestId('add-track').click()
    await page.getByRole('menuitem', { name: 'Drums', exact: true }).click()
    await expect(header(page, 'Synth')).not.toHaveAttribute('aria-current', 'true')
    await expect(page.getByText(x.octave(2), { exact: true })).toBeVisible()
    await page.keyboard.down('KeyA')
    await expect.poll(async () => (await peaks(page))[1]).toBeGreaterThan(0.05)
    expect((await peaks(page))[0]).toBeLessThan(0.001)
    await page.keyboard.up('KeyA')
    await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)
    // The snare, the clap and the open hat answer on the same row.
    for (const code of ['KeyS', 'KeyE', 'KeyU']) {
      await startPeakHold(page)
      await page.keyboard.down(code)
      await expect.poll(() => heldPeak(page, 1), { message: code }).toBeGreaterThan(0.01)
      await page.keyboard.up(code)
      await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)
    }

    // Back on the oscillator: the octave goes back to 4.
    await header(page, 'Synth').getByTestId('track-name').click()
    await expect(page.getByText(x.octave(4), { exact: true })).toBeVisible()

    // A key held while the selection moves ends where it started: the oscillator.
    await page.keyboard.down('KeyA')
    await expect.poll(async () => (await peaks(page))[0]).toBeGreaterThan(0.05)
    await selectedHeader(page).waitFor()
    await page.getByTestId('track-name').nth(1).click()
    await expect(header(page, 'Synth')).not.toHaveAttribute('aria-current', 'true')
    // The page does not release on the selection change: the oscillator still sounds, then ends with the key.
    await page.waitForTimeout(300)
    expect((await peaks(page))[0]).toBeGreaterThan(0.05)
    await page.keyboard.up('KeyA')
    await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)

    // Armed, the oscillator plays with the drums selected; the hint says so.
    const arm = header(page, 'Synth').getByRole('button', { name: x.arm('Synth'), exact: true })
    await arm.click()
    await expect(arm).toHaveAttribute('aria-pressed', 'true')
    await expect(page.getByTestId('keys-target')).toContainText(x.armed)
    await page.keyboard.down('KeyA')
    await expect.poll(async () => Math.min(...(await peaks(page)).slice(0, 2))).toBeGreaterThan(0.01)
    await page.keyboard.up('KeyA')
    await arm.click()
    await expect(arm).toHaveAttribute('aria-pressed', 'false')
    await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)

    // No shortcut fires while playing: with an undoable change made, every playing key and the octave
    // keys leave it in place; Ctrl + Z, which is no playing key, undoes it.
    const mute = header(page, 'Synth').getByRole('button', { name: x.mute('Synth'), exact: true })
    await mute.click()
    await expect(mute).toHaveAttribute('aria-pressed', 'true')
    for (const code of [
      'KeyA',
      'KeyW',
      'KeyS',
      'KeyE',
      'KeyD',
      'KeyF',
      'KeyT',
      'KeyG',
      'KeyY',
      'KeyH',
      'KeyU',
      'KeyJ',
      'KeyK',
      'KeyO',
      'KeyL',
      'KeyP',
      'Semicolon',
      'KeyZ',
      'KeyX',
    ]) {
      await page.keyboard.press(code)
    }
    await expect(mute).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.press('Control+z')
    await expect(mute).toHaveAttribute('aria-pressed', 'false')
    // And no note while the shortcut is held.
    await expect.poll(() => masterPeak(page), { timeout: 8000 }).toBeLessThan(0.001)
    await page.keyboard.down('Control')
    await page.keyboard.down('KeyA')
    await page.waitForTimeout(300)
    expect(await masterPeak(page)).toBeLessThan(0.001)
    await page.keyboard.up('KeyA')
    await page.keyboard.up('Control')

    expect(errors).toEqual([])
  })
}
