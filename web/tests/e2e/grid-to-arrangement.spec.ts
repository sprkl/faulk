import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// AR-5 (#81): a clip or a scene of the launch grid goes to the arrangement from its menu, at the bar of the play
// position. The copy is independent of the grid clip (editing one leaves the other alone), the linked one plays the
// grid clip itself, a scene is one history entry, and the arrangement then sounds. Real worklet, engine and project core.

const SILENCE_DB = -60
const BAR = 3840
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

interface Block {
  id: string
  track: string
  clip: string
  start: number
  length: number
}
const blocks = async (page: Page): Promise<Block[]> =>
  (JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]') as Block[]).sort(
    (a, b) => a.start - b.start || (a.track < b.track ? -1 : 1),
  )

interface View {
  tracks: Record<string, { kind: string; clips: Record<string, { slot: string | null; content: { midi?: { notes: unknown[] } } }> }>
  scenes: Record<string, unknown>
}
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): { project: View } }).__faulkView().project)
const notesOf = (project: View, track: string, clip: string) => project.tracks[track]!.clips[clip]!.content.midi!.notes.length

test('grid to arrangement: a clip copy is independent, a linked one follows the grid, a scene is one undo, and it sounds', async ({ page }) => {
  test.setTimeout(180_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1600, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await engineUp(page)
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
    'the dispatcher hook exists in the dev server only',
  )

  // Two MIDI tracks and two scenes; each track holds a held-note clip in the first scene.
  await run(page, { type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' })
  await run(page, { type: 'scene.create' })
  await run(page, { type: 'scene.create' })
  const first = await view(page)
  const midi = Object.entries(first.tracks)
    .filter(([, t]) => t.kind === 'midi')
    .map(([id]) => id)
  const scenes = Object.keys(first.scenes)
  expect(midi).toHaveLength(2)
  const gridClips: string[] = []
  for (const track of midi) {
    await run(page, { type: 'clip.create', track, at: { at: 'slot', scene: scenes[0] } })
    const id = Object.keys((await view(page)).tracks[track]!.clips)[0]!
    await run(page, { type: 'clip.notes.add', clip: id, notes: [{ start: 0, pitch: 60, length: 3800, velocity: 100 }] })
    gridClips.push(id)
  }

  // Send the first clip, independent: a new clip outside the grid, at bar 1.
  await page.getByTestId('nav-grid').click()
  const cell = page.locator('[data-testid=cell][data-ti="0"][data-si="0"]')
  await cell.hover()
  await page.locator(`[data-slot-clip="${gridClips[0]}"]`).getByTestId('clip-more').click({ force: true })
  await page.getByTestId('clip-menu-send').click()
  await page.getByTestId('nav-arrangement').click()
  await expect.poll(async () => (await blocks(page)).length).toBe(1)
  const [copy] = await blocks(page)
  expect(copy).toMatchObject({ track: midi[0], start: 0, length: BAR })
  expect(copy!.clip).not.toBe(gridClips[0])
  let project = await view(page)
  expect(project.tracks[midi[0]!]!.clips[copy!.clip]!.slot).toBeNull()
  expect(project.tracks[midi[0]!]!.clips[gridClips[0]!]!.slot).toBe(scenes[0])

  // Editing the grid clip leaves the copy alone; editing the copy leaves the grid clip alone.
  await run(page, { type: 'clip.notes.add', clip: gridClips[0], notes: [{ start: 960, pitch: 64, length: 480, velocity: 90 }] })
  project = await view(page)
  expect(notesOf(project, midi[0]!, gridClips[0]!)).toBe(2)
  expect(notesOf(project, midi[0]!, copy!.clip)).toBe(1)
  await run(page, {
    type: 'clip.notes.add',
    clip: copy!.clip,
    notes: [
      { start: 1920, pitch: 67, length: 480, velocity: 90 },
      { start: 2400, pitch: 69, length: 480, velocity: 90 },
    ],
  })
  project = await view(page)
  expect(notesOf(project, midi[0]!, copy!.clip)).toBe(3)
  expect(notesOf(project, midi[0]!, gridClips[0]!)).toBe(2)

  // The arrangement sounds: the seek moved the playhead, Play makes the copy audible.
  await run(page, { type: 'transport.seek', tick: 0 })
  await page.getByTestId('transport').getByRole('button', { name: 'Play', exact: true }).click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await page.getByTestId('transport').getByRole('button', { name: 'Stop', exact: true }).click()
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBeLessThan(SILENCE_DB)

  // Linked, from the middle of bar 3: the placement lands on the bar (7680) and plays the grid clip itself.
  await run(page, { type: 'transport.seek', tick: 2 * BAR + 500 })
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-tick', String(2 * BAR + 500))
  await page.getByTestId('nav-grid').click()
  await cell.hover()
  await page.locator(`[data-slot-clip="${gridClips[0]}"]`).getByTestId('clip-more').click({ force: true })
  await page.getByTestId('clip-menu-send-linked').click()
  await page.getByTestId('nav-arrangement').click()
  await expect.poll(async () => (await blocks(page)).length).toBe(2)
  const linked = (await blocks(page))[1]!
  expect(linked).toMatchObject({ track: midi[0], start: 2 * BAR, clip: gridClips[0] })
  // The grid clip is edited: the linked placement plays the new content (same clip).
  await run(page, { type: 'clip.notes.add', clip: gridClips[0], notes: [{ start: 2000, pitch: 72, length: 100, velocity: 90 }] })
  project = await view(page)
  expect(notesOf(project, midi[0]!, linked.clip)).toBe(3)

  // The linked placement sounds too (the clip is both in the grid and in the arrangement).
  await run(page, { type: 'transport.seek', tick: 2 * BAR })
  await page.getByTestId('transport').getByRole('button', { name: 'Play', exact: true }).click()
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await page.getByTestId('transport').getByRole('button', { name: 'Stop', exact: true }).click()
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBeLessThan(SILENCE_DB)
  await run(page, { type: 'transport.seek', tick: 2 * BAR + 500 })

  // A scene: one placement per track at bar 3 (the linked one is carved by the first track's new copy: last wins),
  // one history entry, so one undo.
  await page.getByTestId('nav-grid').click()
  const head = page.locator('[data-testid=scene-head]').first()
  await head.hover()
  await head.getByTestId('scene-more').click({ force: true })
  await page.getByTestId('scene-send').click()
  await page.getByTestId('nav-arrangement').click()
  await expect.poll(async () => (await blocks(page)).length).toBe(3)
  const afterScene = await blocks(page)
  const atBar3 = afterScene.filter((b) => b.start === 2 * BAR)
  expect(atBar3.map((b) => b.track).sort()).toEqual([...midi].sort())
  expect(atBar3.every((b) => !gridClips.includes(b.clip))).toBe(true)
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await blocks(page)).length).toBe(2)
  expect((await blocks(page))[1]).toMatchObject({ clip: gridClips[0], start: 2 * BAR })

  // An empty scene cannot be sent.
  await page.getByTestId('nav-grid').click()
  const empty = page.locator('[data-testid=scene-head]').nth(1)
  await empty.hover()
  await empty.getByTestId('scene-more').click({ force: true })
  await expect(page.getByTestId('scene-send')).toBeDisabled()
  await page.keyboard.press('Escape')

  expect(seen.errors).toEqual([])
})

test('sending a grid clip onto a taken place is refused, says so, and leaves the neighbour whole (#860)', async ({ page }) => {
  const seen = watch(page)
  await page.setViewportSize({ width: 1600, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await engineUp(page)
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
    'the dispatcher hook exists in the dev server only',
  )

  await run(page, { type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' })
  await run(page, { type: 'scene.create' })
  const first = await view(page)
  const track = Object.entries(first.tracks).find(([, t]) => t.kind === 'midi')![0]
  const scene = Object.keys(first.scenes)[0]
  await run(page, { type: 'clip.create', track, at: { at: 'slot', scene } })
  const gridClip = Object.keys((await view(page)).tracks[track]!.clips)[0]!
  await run(page, { type: 'clip.notes.add', clip: gridClip, notes: [{ start: 0, pitch: 60, length: 480, velocity: 100 }] })
  // A clip of the arrangement already sits at bar 1, where the playhead is.
  await run(page, { type: 'clip.create', track, at: { at: 'arrangement', start: 0 } })
  await page.getByTestId('nav-arrangement').click()
  await expect.poll(async () => (await blocks(page)).length).toBe(1)
  const before = await blocks(page)

  await page.getByTestId('nav-grid').click()
  await page.locator('[data-testid=cell][data-ti="0"][data-si="0"]').hover()
  await page.locator(`[data-slot-clip="${gridClip}"]`).getByTestId('clip-more').click({ force: true })
  await page.getByTestId('clip-menu-send').click()
  await expect(page.getByTestId('refusal-toast')).toContainText(/taken/)
  await page.getByTestId('nav-arrangement').click()
  expect(await blocks(page)).toEqual(before)

  expect(seen.errors).toEqual([])
})
