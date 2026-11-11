import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// AU-4 (#113): an imported audio file plays. The user makes an audio track (offered by the add-track menu), imports a WAV
// in its Clips panel, puts it in the grid and clicks its cell: the engine's audio clip player reads the file through the
// disk planner (the prepare Worker reads the draft's `blobs/`), resamples it to the context's rate, and the output meter
// moves; a stop silences it. Through the real worklet and the shipped engine module.

test.describe.configure({ timeout: 180_000 })

const FIXTURES = fileURLToPath(new URL('../../../crates/audio-import/tests/fixtures/', import.meta.url))
const SILENCE_DB = -60

type View = { project: { tracks: Record<string, { kind: string; clips: Record<string, { slot: string | null; content: { audio?: unknown } }> }> } }
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const meterDb = async (page: Page) => Number(await page.getByTestId('top-meter').getAttribute('aria-valuenow'))

test('an imported WAV put in the grid plays when its cell is clicked, and a stop silences it', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  test.skip((await hooks(page)) !== 'function', 'the command hooks exist in the dev server only')
  // The add-track menu offers an audio track.
  await page.getByTestId('add-track').click()
  await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Audio track' }).click()
  const track = Object.entries((await view(page)).project.tracks).find(([, t]) => t.kind === 'audio')![0]
  await run(page, { type: 'view.select_track', track })
  await run(page, { type: 'scene.create', name: 'A' })
  await page.getByTestId('panel-clips').click()
  // Web mode: no word about Boost.
  await expect(page.getByTestId('audio-boost-folder')).toHaveCount(0)
  await expect(page.getByTestId('audio-boost-show-folder')).toHaveCount(0)
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('audio-import-button').click()
  await (await chooser).setFiles(`${FIXTURES}sine-s16.wav`)
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  const clip = Object.entries((await view(page)).project.tracks[track]!.clips).find(([, c]) => c.content.audio)![0]
  // Looped over the whole file (0.5 s at 44.1 kHz), so it sounds until stopped.
  await run(page, { type: 'clip.audio.set_loop', clip, looping: { start: 0, end: 22_050 } })
  await page.getByTestId('pool-grid').click()
  await expect.poll(async () => (await view(page)).project.tracks[track]!.clips[clip]!.slot).not.toBeNull()
  const cell = page.locator(`[data-cell][data-clip="${clip}"]`)
  await expect(cell).toBeEnabled()
  await expect(cell).toHaveAttribute('aria-label', /audio clip, stopped/)
  expect(await meterDb(page)).toBe(SILENCE_DB)
  await cell.click()
  await expect(cell).toHaveAttribute('data-state', 'playing', { timeout: 15_000 })
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBeGreaterThan(-30)
  await run(page, { type: 'transport.stop' })
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBe(SILENCE_DB)
  await expect(cell).not.toHaveAttribute('data-state', 'playing')
  expect(errors).toEqual([])
})
