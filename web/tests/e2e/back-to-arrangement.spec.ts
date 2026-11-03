import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #314: a track launched from the grid does not follow its timeline any more, and Stop does not give
// it back. The transport bar's "Back to arrangement" button (one `launch.back_to_arrangement`) is lit
// while a track is held, and the arrangement view marks the track with its own return
// (`launch.back_track_to_arrangement`). Real worklet, engine and project core; the project is built
// through the page's dev hook (dev server only).

const SILENCE_DB = -60
const BEAT = 960
const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
const meterDb = async (page: Page) => {
  const peak = Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
  return peak > 0 ? 20 * Math.log10(peak) : -Infinity
}
const transport = (page: Page, name: string) => page.getByTestId('transport').getByRole('button', { name, exact: true })
const back = (page: Page) => page.getByTestId('transport-back-to-arrangement')

test('a track launched from the grid stays out of its timeline after Stop until it is given back', async ({ page }) => {
  test.setTimeout(240_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await engineUp(page)
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
    'the dispatcher hook exists in the dev server only',
  )

  // One MIDI track: four bars of the arrangement with a note on every beat, and a looping clip in the first scene of the grid.
  await run(page, { type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' })
  await run(page, { type: 'scene.create' })
  const view = () =>
    page.evaluate(
      () =>
        (
          window as unknown as {
            __faulkView(): { project: { tracks: Record<string, { clips: Record<string, { slot: string | null }> }>; scenes: Record<string, unknown> } }
          }
        ).__faulkView().project,
    )
  const project = await view()
  const track = Object.keys(project.tracks)[0]!
  const scene = Object.keys(project.scenes)[0]!
  await run(page, { type: 'clip.create', track, at: { at: 'arrangement', start: 0 }, length: 16 * BEAT })
  await run(page, { type: 'clip.create', track, at: { at: 'slot', scene } })
  const clips = (await view()).tracks[track]!.clips
  const gridClip = Object.keys(clips).find((id) => clips[id]!.slot !== null)!
  const arrClip = Object.keys(clips).find((id) => id !== gridClip)!
  const notes = (pitch: number, count: number) => Array.from({ length: count }, (_, i) => ({ start: i * BEAT, pitch, length: BEAT - 100, velocity: 100 }))
  await run(page, { type: 'clip.notes.add', clip: arrClip, notes: notes(60, 16) })
  await run(page, { type: 'clip.notes.add', clip: gridClip, notes: [{ start: 0, pitch: 72, length: 3800, velocity: 100 }] })
  await run(page, { type: 'clip.set_loop', clip: gridClip, looping: { start: 0, end: 3840 } })

  // Nothing is held: the button is off.
  await expect(back(page)).toHaveAttribute('aria-pressed', 'false')

  // The arrangement sounds.
  await transport(page, 'Play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)

  // Launch the grid clip: the track leaves its timeline, the button lights.
  await run(page, { type: 'launch.clip', clip: gridClip })
  await expect(back(page)).toHaveAttribute('aria-pressed', 'true', { timeout: 15_000 })

  // Stop does not give the track back: the button stays lit, and the next Play is silent (the timeline is not followed).
  await transport(page, 'Stop').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'stopped')
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBeLessThan(SILENCE_DB)
  await expect(back(page)).toHaveAttribute('aria-pressed', 'true')
  await transport(page, 'Play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  await page.waitForTimeout(2500)
  expect(await meterDb(page)).toBeLessThan(SILENCE_DB)
  await expect(back(page)).toHaveAttribute('aria-pressed', 'true')

  // The button gives the track back: it goes off, and the arrangement sounds again.
  await back(page).click()
  await expect(back(page)).toHaveAttribute('aria-pressed', 'false', { timeout: 10_000 })
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)

  // The same, from the arrangement view: the held track is marked and has its own return.
  await run(page, { type: 'launch.clip', clip: gridClip })
  await expect(back(page)).toHaveAttribute('aria-pressed', 'true', { timeout: 15_000 })
  await transport(page, 'Stop').click()
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBeLessThan(SILENCE_DB)
  await page.getByTestId('nav-arrangement').click()
  await expect(page.getByTestId('arr-back-track')).toHaveCount(1)
  await page.getByTestId('arr-back-track').click()
  await expect(page.getByTestId('arr-back-track')).toHaveCount(0)
  await expect(back(page)).toHaveAttribute('aria-pressed', 'false')
  await transport(page, 'Play').click()
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await transport(page, 'Stop').click()
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBeLessThan(SILENCE_DB)

  expect(seen.errors).toEqual([])
})
