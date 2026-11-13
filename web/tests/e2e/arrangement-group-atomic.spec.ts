import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #867: a multi-clip keyboard action is all or nothing. A MIDI clip and an audio clip are selected together and
// Alt+Shift+Right asks both to grow: the core refuses the audio one (its end stops at its last frame), so the MIDI clip
// must not have grown either, the history holds no new entry, and one message says so. Real worklet, engine and core.

test.describe.configure({ timeout: 240_000 })
test.use({ storageState: { cookies: [], origins: [] } })

const FIXTURES = fileURLToPath(new URL('../../../crates/audio-import/tests/fixtures/', import.meta.url))

type View = { project: { tracks: Record<string, { kind: string; clips: Record<string, unknown>; placements: Record<string, unknown> }> } }
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
const history = (page: Page) => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[]; redo: unknown[] } }).__faulkHistory())
const placements = async (page: Page) => JSON.stringify(Object.values((await view(page)).project.tracks).map((t) => t.placements))

test('Alt+Shift+Right on a selection where the core refuses one clip changes none of them (#867)', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function', 'the dev server only')

  // A MIDI clip of one bar on the first track, an audio clip (the reference file) placed at bar 1 of an audio track.
  await page.getByTestId('add-track').click()
  await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Audio track' }).click()
  const audio = Object.entries((await view(page)).project.tracks).find(([, t]) => t.kind === 'audio')![0]
  await run(page, { type: 'view.select_track', track: audio })
  await page.getByTestId('panel-clips').click()
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('audio-import-button').click()
  await (await chooser).setFiles(`${FIXTURES}sine-s16.wav`)
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  const blocks = async (): Promise<{ id: string; row: number; start: number; end: number }[]> =>
    JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')
  // Audio clip A at tick 0 cut to half a beat, audio clip B right after it (tick 480): A cannot grow without carving B.
  await page.getByTestId('pool-place').first().click()
  await expect.poll(async () => (await blocks()).length).toBe(1)
  const a = (await blocks())[0]!
  expect(await run(page, { type: 'arrangement.resize', placement: a.id, edge: 'end', to: 480 })).toBe(true)
  const clip = Object.keys((await view(page)).project.tracks[audio]!.clips)[0]!
  expect(await run(page, { type: 'arrangement.place', clip, start: 480 })).toBe(true)
  await expect.poll(async () => (await blocks()).length).toBe(2)

  // A MIDI clip of one bar on the first track.
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.dblclick(box.x + 1920 * ppt, box.y + 28 + 44 / 2)
  await expect.poll(async () => (await blocks()).length).toBe(3)
  const midi = (await blocks()).find((b) => b.row === 0)!

  // Select the MIDI clip and A: a click on one, a Shift+click on the other.
  const cell = (id: string) => page.locator(`[data-testid=arr-item][data-block="${id}"]`)
  const at = (b: { row: number; start: number }, dx: number) => ({ x: box.x + (b.start + dx) * ppt, y: box.y + 28 + b.row * 44 + 44 / 2 })
  await page.mouse.click(at(midi, 480).x, at(midi, 480).y)
  await page.keyboard.down('Shift')
  await page.mouse.click(at(a, 240).x, at(a, 240).y)
  await page.keyboard.up('Shift')
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-selected-count', '2')

  const before = await placements(page)
  const entries = await history(page)
  await cell(midi.id).focus()
  await page.keyboard.press('Alt+Shift+ArrowRight')

  // One message, nothing changed: not the MIDI clip, not the audio one; no history entry; the history is not stuck in a group.
  await expect(page.getByTestId('arr-said')).toHaveText(/no clip changed/)
  await expect(page.getByTestId('refusal-toast')).toHaveCount(1)
  expect(await placements(page)).toBe(before)
  const after = await history(page)
  expect(after.undo).toHaveLength(entries.undo.length)
  expect(after.redo).toHaveLength(entries.redo.length)
  expect(await run(page, { type: 'history.undo' })).toBe(true)
  expect(errors).toEqual([])
})
