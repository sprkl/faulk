import { expect, test, type Page } from '@playwright/test'
import { fakeWebMidi, watch } from './support'

// #365: typing a value into a continuous control, in a real browser. Enter on a focused control or a
// click on its readout opens a field; Enter applies by the existing command (one history entry),
// Escape cancels, an unreadable text changes nothing and says so. Real worklet, engine and project core.

test.describe.configure({ mode: 'serial' })

async function start(page: Page, lang: 'en' | 'fr' = 'en') {
  await fakeWebMidi(page)
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  await page.getByRole('radio', { name: 'E2E Keys' }).check()
  await expect(page.getByTestId('track-header')).toHaveCount(1)
}

const field = (page: Page) => page.getByTestId('value-input')

/** Types `text` in the open field and presses Enter. */
async function enter(page: Page, text: string) {
  await expect(field(page)).toBeFocused()
  await field(page).fill(text)
  await page.keyboard.press('Enter')
}

test('a device knob: Enter opens the field, units are read, Escape cancels, one undo', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const attack = page.getByTestId('param-attack').getByRole('slider')
  const factory = (await attack.getAttribute('aria-valuetext'))!

  await attack.focus()
  await page.keyboard.press('Enter')
  await expect(field(page)).toHaveValue(factory)
  await enter(page, '300 ms')
  await expect(field(page)).toHaveCount(0)
  await expect(attack).toHaveAttribute('aria-valuetext', '300 ms')
  await expect(attack).toBeFocused()
  // One entry: one undo takes the typed value back whole.
  await page.keyboard.press('Control+z')
  await expect(attack).toHaveAttribute('aria-valuetext', factory)
  await page.keyboard.press('Control+y')
  await expect(attack).toHaveAttribute('aria-valuetext', '300 ms')

  // Seconds, and a value out of range comes back to the bound.
  await page.getByTestId('param-attack').getByTestId('value-readout').click()
  await enter(page, '1.5 s')
  await expect(attack).toHaveAttribute('aria-valuetext', '1.50 s')
  await attack.focus()
  await page.keyboard.press('Enter')
  await enter(page, '999')
  await expect(attack).toHaveAttribute('aria-valuetext', '30.00 s')

  // Escape cancels.
  await attack.focus()
  await page.keyboard.press('Enter')
  await field(page).fill('5 ms')
  await page.keyboard.press('Escape')
  await expect(field(page)).toHaveCount(0)
  await expect(attack).toHaveAttribute('aria-valuetext', '30.00 s')
  await expect(attack).toBeFocused()

  // An unreadable text changes nothing, says so and keeps the field; a foreign unit is unreadable too.
  await page.keyboard.press('Enter')
  await enter(page, '5 Hz')
  await expect(page.getByTestId('value-error')).toBeVisible()
  await expect(field(page)).toHaveAttribute('aria-invalid', 'true')
  await expect(attack).toHaveAttribute('aria-valuetext', '30.00 s')
  // The field takes every key: letters and Space type, they play no note and do not start the transport.
  await field(page).fill('')
  await page.keyboard.type('a s d')
  await expect(field(page)).toHaveValue('a s d')
  await expect(page.getByTestId('voices')).not.toContainText(/\b1\b/)
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toHaveAttribute('aria-pressed', 'false')
  // Losing the focus with a readable text applies it.
  await field(page).fill('250 ms')
  await page.mouse.click(2, 2)
  await expect(field(page)).toHaveCount(0)
  await expect(attack).toHaveAttribute('aria-valuetext', '250 ms')

  // The double click is still the reset.
  await attack.dblclick()
  await expect(attack).toHaveAttribute('aria-valuetext', factory)
  expect(errors).toEqual([])
})

test('the mixer: fader and pan take a typed value, silence included', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('nav-mixer').click()
  const strip = page.getByTestId('mixer-strip').first()
  const fader = strip.getByTestId('mixer-fader')
  const pan = strip.getByTestId('mixer-pan')

  await fader.focus()
  await page.keyboard.press('Enter')
  await expect(field(page)).toHaveValue('0.0 dB')
  await enter(page, '-6')
  await expect(fader).toHaveAttribute('aria-valuetext', '−6.0 dB')
  await page.keyboard.press('Control+z')
  await expect(fader).toHaveAttribute('aria-valuetext', '0.0 dB')

  // The readout under the strip opens it; silence reads back as itself.
  await strip.getByTestId('mixer-gain').click()
  await enter(page, '-inf')
  await expect(fader).toHaveAttribute('aria-valuetext', '−∞ dB')
  await fader.focus()
  await page.keyboard.press('Enter')
  await expect(field(page)).toHaveValue('−∞ dB')
  await page.keyboard.press('Enter')
  await expect(fader).toHaveAttribute('aria-valuetext', '−∞ dB')
  await fader.dblclick()
  await expect(fader).toHaveAttribute('aria-valuetext', '0.0 dB')

  await strip.getByTestId('mixer-pan-readout').click()
  await expect(field(page)).toHaveValue('C')
  await enter(page, 'L30')
  await expect(pan).toHaveAttribute('aria-valuetext', 'L30')
  await page.keyboard.press('Control+z')
  await expect(pan).toHaveAttribute('aria-valuetext', 'C')
  await pan.focus()
  await page.keyboard.press('Enter')
  await enter(page, '500')
  await expect(pan).toHaveAttribute('aria-valuetext', 'R100')
  expect(errors).toEqual([])
})

test('the track list: volume and pan take a typed value', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const header = page.getByTestId('track-header').first()
  const volume = header.getByRole('slider', { name: /^Volume of/ })
  const pan = header.getByRole('slider', { name: /^Pan of/ })

  await volume.focus()
  await page.keyboard.press('Enter')
  await enter(page, '-12.5 dB')
  await expect(volume).toHaveAttribute('aria-valuetext', '−12.5 dB')
  // Enter on the control opens the field only: the track row does not react to it.
  await expect(header.locator('input.rename')).toHaveCount(0)
  await page.keyboard.press('Control+z')
  await expect(volume).toHaveAttribute('aria-valuetext', '0.0 dB')

  await header.getByTestId('track-pan-readout').click()
  await enter(page, 'R25')
  await expect(pan).toHaveAttribute('aria-valuetext', 'R25')
  await page.keyboard.press('Control+z')
  await expect(pan).toHaveAttribute('aria-valuetext', 'C')
  expect(errors).toEqual([])
})

test('French: the field is named and complains in French', async ({ page }) => {
  const { errors } = watch(page)
  await start(page, 'fr')
  const attack = page.getByTestId('param-attack').getByRole('slider')
  await attack.focus()
  await page.keyboard.press('Enter')
  await expect(field(page)).toHaveAttribute('aria-label', /^Valeur de /)
  await enter(page, 'beaucoup')
  await expect(page.getByTestId('value-error')).toHaveText('Valeur invalide')
  await enter(page, '120 ms')
  await expect(attack).toHaveAttribute('aria-valuetext', '120 ms')
  expect(errors).toEqual([])
})
