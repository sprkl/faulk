import { expect, test } from '@playwright/test'
import { engineUp, watch } from './support'

// #806: after the Master's devices were shown (Mixer, "Devices" of the Master strip), choosing a track in the arrangement
// brings the Devices panel back to that track, even when it is the track that was selected when the Master was chosen.
// Choosing the Master again shows the Master.

test('devices panel follows the track chosen in the arrangement after the Master, and the Master when chosen again', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: 'Drums', exact: true }).click()

  await page.getByTestId('nav-arrangement').click()
  const devices = page.getByTestId('devices')
  const heads = page.getByTestId('track-header')
  await expect(heads).toHaveCount(2)
  const drums = heads.nth(1)

  for (const how of ['header', 'row'] as const) {
    // The Drums track is the selected one when the Master is chosen.
    await page.getByTestId('nav-mixer').click()
    await page.getByTestId('mixer-master').getByTestId('mixer-master-devices').click()
    await expect(devices).toHaveAttribute('data-target', 'master')
    await page.getByTestId('nav-arrangement').click()

    if (how === 'header') await drums.getByTestId('arr-track').click()
    else {
      const head = (await drums.boundingBox())!
      const stage = (await page.getByTestId('arr-stage').boundingBox())!
      await page.mouse.click(stage.x + stage.width - 20, head.y + head.height / 2)
    }
    await page.getByTestId('panel-devices').click()
    await expect(devices).toBeVisible()
    await expect(devices).not.toHaveAttribute('data-target', 'master')
    await expect(page.getByTestId('device-chain')).toHaveAttribute('aria-label', 'Devices of Track 2')
  }

  // Choosing the Master explicitly still shows it.
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('mixer-master').getByTestId('mixer-master-devices').click()
  await expect(devices).toHaveAttribute('data-target', 'master')
  expect(errors).toEqual([])
})
