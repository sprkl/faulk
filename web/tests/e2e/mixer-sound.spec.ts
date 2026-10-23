import { expect, test, type Page } from '@playwright/test'
import { fakeWebMidi, watch } from './support'

// MX-1 (#90): the track controls act on the sound in the real page. The hidden `feedback`
// element carries the engine's report (the meters, after the fader): the volume lowers the
// level by what it says, mute and solo silence the track, and a gesture leaves no error.

test.describe.configure({ mode: 'serial' })

const feedback = (page: Page) => page.getByTestId('feedback')
const master = async (page: Page) => Number(await feedback(page).getAttribute('data-master-peak'))
const send = (page: Page, bytes: number[]) =>
  page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

test('volume, mute and solo of a track change what the engine sounds', async ({ page }) => {
  const { errors } = watch(page)
  await fakeWebMidi(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  await page.getByRole('radio', { name: 'E2E Keys' }).check()
  await expect(page.getByTestId('track-header')).toHaveCount(1)
  const synth = page.getByTestId('track-header').first()

  // A held note: the level at unity.
  await send(page, [0x90, 69, 127])
  await expect.poll(() => master(page)).toBeGreaterThan(0.05)
  await page.waitForTimeout(500)
  const unity = await master(page)

  // -6 dB on the fader: the level follows (half, within the report's resolution).
  await synth.getByRole('slider', { name: 'Volume of Synth', exact: true }).fill('-6')
  await expect.poll(async () => (await master(page)) / unity, { timeout: 5000 }).toBeGreaterThan(0.45)
  await expect.poll(async () => (await master(page)) / unity).toBeLessThan(0.56)
  await synth.getByRole('slider', { name: 'Volume of Synth', exact: true }).dblclick()
  await expect.poll(async () => (await master(page)) / unity, { timeout: 5000 }).toBeGreaterThan(0.9)

  // Mute silences it, unmute brings it back.
  const mute = synth.getByRole('button', { name: 'Mute Synth', exact: true })
  await mute.click()
  await expect.poll(() => master(page), { timeout: 5000 }).toBeLessThan(0.001)
  await mute.click()
  await expect.poll(() => master(page), { timeout: 5000 }).toBeGreaterThan(0.05)

  // Solo on another track silences this one; solo on this one brings it back (cumulative).
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: 'Drums', exact: true }).click()
  await expect(page.getByTestId('track-header')).toHaveCount(2)
  const soloOf = (name: string) => page.getByRole('button', { name: `Solo ${name}`, exact: true })
  await soloOf('Track 2').click()
  await expect.poll(() => master(page), { timeout: 5000 }).toBeLessThan(0.001)
  await soloOf('Synth').click()
  await expect.poll(() => master(page), { timeout: 5000 }).toBeGreaterThan(0.05)
  await soloOf('Synth').click()
  await soloOf('Track 2').click()
  await expect.poll(() => master(page), { timeout: 5000 }).toBeGreaterThan(0.05)
  await send(page, [0x80, 69, 0])
  expect(errors).toEqual([])
})
