import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #470, stage 2: the piano roll follows the clip selected in the arrangement, opens a clip it never showed on that clip's
// notes, and draws the notes of the other tracks hollow on screen. Real worklet, engine and project core; the clips are
// made through the dev server's command hook, the gestures through the pointer.

test.use({ storageState: { cookies: [], origins: [] } })

const RULER = 28
const LANE = 44
const ROW = 16

const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
type View = { project: { tracks: Record<string, { kind: string; clips: Record<string, unknown> }> }; session: { selected_track: string | null } }
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())
const blocks = async (page: Page): Promise<{ id: string; row: number; clip: string; start: number; length: number }[]> =>
  JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')
const roll = (page: Page) => page.getByTestId('piano-roll')

async function start(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  test.skip((await hooks(page)) !== 'function', 'the clips are made through the hook of the dev server')
  await page.getByTestId('nav-arrangement').click()
  await expect.poll(async () => Number(await page.getByTestId('arr-stage').getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
}

/** A clip of one bar placed at `start` on `track`, with these notes; returns its id. */
async function placed(page: Page, track: string, startTick: number, notes: { start: number; pitch: number }[]) {
  const before = new Set(Object.keys((await view(page)).project.tracks[track]!.clips))
  await run(page, { type: 'clip.create', track, at: { at: 'arrangement', start: startTick } })
  const clip = Object.keys((await view(page)).project.tracks[track]!.clips).find((id) => !before.has(id))!
  await run(page, { type: 'clip.notes.add', clip, notes: notes.map((n) => ({ ...n, length: 480, velocity: 100 })) })
  return clip
}

/** Clicks the middle of a clip of the arrangement. */
async function clickBlock(page: Page, clip: string) {
  const block = (await blocks(page)).find((b) => b.clip === clip)!
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.click(box.x + (block.start + block.length / 2) * ppt, box.y + RULER + block.row * LANE + LANE / 2)
}

/** Whether the row of `pitch` is inside the roll's stage. */
async function rowInSight(page: Page, pitch: number) {
  const stage = page.getByTestId('roll-stage')
  const top = Number(await stage.getAttribute('data-scroll-y'))
  const height = (await stage.boundingBox())!.height
  const rowTop = (127 - pitch) * ROW - top + RULER
  return rowTop >= RULER && rowTop + ROW <= height
}

test('the piano roll follows the clip selected in the arrangement, and opens a clip it never showed on its notes', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const track = Object.entries((await view(page)).project.tracks).find(([, t]) => t.kind === 'midi')![0]
  // A low clip, then a high one: far apart, so that the second can only be in sight if the roll opens on it.
  const low = await placed(page, track, 0, [{ start: 0, pitch: 36 }])
  const high = await placed(page, track, 3840, [{ start: 960, pitch: 100 }])
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-block-count', '2')

  await clickBlock(page, low)
  await page.getByTestId('panel-piano-roll').click()
  await expect(roll(page)).toHaveAttribute('data-clip', low)
  await expect.poll(() => rowInSight(page, 36)).toBe(true)
  expect(await rowInSight(page, 100)).toBe(false)

  // Selecting the other clip in the arrangement: the roll follows, and shows this clip's note at its first display.
  await clickBlock(page, high)
  await expect(roll(page)).toHaveAttribute('data-clip', high)
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  await expect.poll(() => rowInSight(page, 100)).toBe(true)

  // And back.
  await clickBlock(page, low)
  await expect(roll(page)).toHaveAttribute('data-clip', low)
  expect(errors).toEqual([])
})

test('the notes of another track are drawn hollow under the clip, and the toggle hides them', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const first = Object.entries((await view(page)).project.tracks).find(([, t]) => t.kind === 'midi')![0]
  await page.getByTestId('add-track').click()
  await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Oscillator', exact: true }).click()
  await expect(page.getByTestId('track-header')).toHaveCount(2)
  const second = Object.keys((await view(page)).project.tracks).find((id) => id !== first)!
  const mine = await placed(page, first, 0, [{ start: 0, pitch: 60 }])
  // At the same time on the other track, two semitones up.
  await placed(page, second, 0, [{ start: 960, pitch: 62 }])

  await clickBlock(page, mine)
  await page.getByTestId('panel-piano-roll').click()
  await expect(roll(page)).toHaveAttribute('data-clip', mine)
  await expect.poll(() => rowInSight(page, 62)).toBe(true)
  const stage = page.getByTestId('roll-stage')
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const top = Number(await stage.getAttribute('data-scroll-y'))
  // The ghost's outline (its top edge) and its middle, read from the canvas of the roll, in CSS pixels of the stage.
  const rowTop = RULER + (127 - 62) * ROW - top
  const at = { edge: { x: 960 * ppt + 20, y: rowTop + 1.5 }, middle: { x: 960 * ppt + 20, y: rowTop + ROW / 2 } }
  const pixels = () =>
    stage.evaluate((el, points) => {
      const canvas = el.querySelector('canvas')!
      const scale = canvas.width / canvas.getBoundingClientRect().width
      const ctx = canvas.getContext('2d')!
      const read = (p: { x: number; y: number }) => Array.from(ctx.getImageData(Math.floor(p.x * scale), Math.floor(p.y * scale), 1, 1).data)
      return { edge: read(points.edge), middle: read(points.middle) }
    }, at)
  const others = page.getByTestId('roll-others')
  await expect(others).toHaveAttribute('aria-pressed', 'true')
  const shown = await pixels()
  await others.click()
  await expect(others).toHaveAttribute('aria-pressed', 'false')
  await expect
    .poll(async () => JSON.stringify((await pixels()).edge) !== JSON.stringify(shown.edge), { message: 'the outline is drawn, then hidden' })
    .toBe(true)
  // Hollow: the middle of the ghost is the row behind it, shown or not.
  expect((await pixels()).middle).toEqual(shown.middle)
  // Neutral: the outline is a grey (its three channels close), not the track's hue.
  const [r, g, b] = shown.edge
  expect(Math.max(r!, g!, b!) - Math.min(r!, g!, b!)).toBeLessThan(24)
  expect(errors).toEqual([])
})
