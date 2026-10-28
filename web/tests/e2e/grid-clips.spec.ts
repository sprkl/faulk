import { expect, test, type Locator, type Page } from '@playwright/test'
import { watch } from './support'

// #295: delete, duplicate, move and rename a clip from its cell, in a real browser with the real
// engine and project core. The project is built through the page's dev hook (dev server only); every
// gesture is then checked for what it changes and for being one undo.

const SILENCE_DB = -60
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
/** The cell at a track column and a scene row (0 first). */
const at = (page: Page, track: number, scene: number) => page.locator(`[data-testid=cell][data-ti="${track}"][data-si="${scene}"]`)
const clipOf = (cell: Locator) => cell.getAttribute('data-clip')

/** Presses on a cell, moves to another by steps and lets go there (the keys held are those of `modifier`). */
async function drag(page: Page, from: Locator, to: Locator, modifier?: 'Alt') {
  const a = (await from.boundingBox())!
  const b = (await to.boundingBox())!
  await page.mouse.move(a.x + 30, a.y + a.height / 2)
  await page.mouse.down()
  await page.mouse.move(a.x + 40, a.y + a.height / 2, { steps: 3 })
  await page.mouse.move(b.x + 30, b.y + b.height / 2, { steps: 8 })
  if (modifier) await page.keyboard.down(modifier)
  await page.mouse.move(b.x + 31, b.y + b.height / 2)
  return {
    drop: async () => {
      await page.mouse.up()
      if (modifier) await page.keyboard.up(modifier)
    },
  }
}

test('grid: delete, rename, duplicate, copy and move a clip from its cell', async ({ page }) => {
  test.setTimeout(240_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  test.skip(
    (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
    'the dispatcher hook exists in the dev server only',
  )

  // Columns: the first MIDI track, a second MIDI track, an audio track. Rows: four scenes.
  await run(page, { type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' })
  await run(page, { type: 'track.create', kind: 'audio' })
  for (let i = 0; i < 4; i++) await run(page, { type: 'scene.create' })
  const view = () =>
    page.evaluate(
      () =>
        (
          window as unknown as {
            __faulkView(): {
              project: {
                tracks: Record<string, { kind: string; clips: Record<string, { slot: string | null; name: string; content: { midi?: { notes: unknown[] } } }> }>
                scenes: Record<string, unknown>
              }
            }
          }
        ).__faulkView().project,
    )
  const project = await view()
  const midi = Object.entries(project.tracks)
    .filter(([, t]) => t.kind === 'midi')
    .map(([id]) => id)
  const scenes = Object.keys(project.scenes)
  expect(midi).toHaveLength(2)
  // A clip with a held note in the first cell of the first MIDI track.
  await run(page, { type: 'clip.create', track: midi[0], at: { at: 'slot', scene: scenes[0] } })
  const clipId = Object.keys((await view()).tracks[midi[0]!]!.clips)[0]!
  await run(page, { type: 'clip.notes.add', clip: clipId, notes: [{ start: 0, pitch: 60, length: 3800, velocity: 100 }] })
  await run(page, { type: 'clip.set_loop', clip: clipId, looping: { start: 0, end: 3840 } })
  await page.getByTestId('nav-grid').click()
  await expect(page.getByTestId('grid')).toBeVisible()
  const origin = at(page, 0, 0)

  // The default name is stored in the project at creation, and every view shows it.
  await expect(origin).toContainText('Clip 1')
  expect((await view()).tracks[midi[0]!]!.clips[clipId]!.name).toBe('Clip 1')

  // Delete: a playing clip stops cleanly, the cell empties, one undo brings the clip back with its notes.
  await origin.click()
  await expect(origin).toHaveAttribute('data-state', 'playing', { timeout: 10_000 })
  await expect.poll(() => meterDb(page), { timeout: 5_000 }).toBeGreaterThan(SILENCE_DB)
  await origin.focus()
  await page.keyboard.press('Delete')
  await expect(origin).toHaveAttribute('data-state', 'empty')
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeLessThan(SILENCE_DB)
  await page.keyboard.press('Control+z')
  await expect(origin).toHaveAttribute('data-clip', clipId)
  await expect(origin).toHaveAttribute('data-state', 'idle')
  expect((await view()).tracks[midi[0]!]!.clips[clipId]!.content.midi!.notes).toHaveLength(1)
  await at(page, 0, 0).focus()
  await page.keyboard.press('Backspace')
  await expect(origin).toHaveAttribute('data-state', 'empty')
  await page.keyboard.press('Control+z')
  await expect(origin).toHaveAttribute('data-clip', clipId)

  // Rename with F2: Enter commits (the name is in the cell, the aria-label and the roll's title), Escape cancels, undo reverts.
  await origin.focus()
  await page.keyboard.press('F2')
  const field = page.getByTestId('clip-rename')
  await expect(field).toBeFocused()
  await field.fill('Verse riff')
  await page.keyboard.press('Enter')
  await expect(origin).toContainText('Verse riff')
  await expect(origin).toHaveAttribute('aria-label', /Verse riff/)
  await expect(origin).toBeFocused()
  await page.getByTestId('edit-clip').first().click({ force: true })
  await expect(page.getByTestId('roll-clip-name')).toHaveText('Verse riff')
  await page.getByTestId('nav-grid').click()
  await origin.focus()
  await page.keyboard.press('F2')
  await field.fill('Discarded')
  await page.keyboard.press('Escape')
  await expect(origin).toContainText('Verse riff')
  await page.keyboard.press('Control+z')
  await expect(origin).toContainText('Clip 1')
  // A double-click still opens the clip in the piano roll, it does not rename.
  await origin.dblclick()
  await expect(page.getByTestId('clip-rename')).toHaveCount(0)
  await expect(page.getByTestId('piano-roll')).toBeVisible()

  // The menu: Rename, Duplicate (next free cell below), Delete.
  await page.getByTestId('nav-grid').click()
  // The double-click's first click launched the clip: stop it.
  await page.getByTestId('stop-all').click()
  await expect(origin).toHaveAttribute('data-state', 'idle', { timeout: 10_000 })
  await origin.hover()
  await page.getByTestId('clip-more').first().click()
  await page.getByTestId('clip-menu-rename').click()
  await page.getByTestId('clip-rename').fill('Menu name')
  await page.keyboard.press('Enter')
  await expect(origin).toContainText('Menu name')
  await origin.hover()
  await page.getByTestId('clip-more').first().click()
  await page.getByTestId('clip-menu-duplicate').click()
  const below = at(page, 0, 1)
  await expect(below).toHaveAttribute('data-clip', /.+/)
  await expect(below).toContainText('Menu name')
  expect(await clipOf(below)).not.toBe(clipId)
  await page.keyboard.press('Control+z')
  await expect(below).toHaveAttribute('data-state', 'empty')
  await page.keyboard.press('Control+z')
  await expect(origin).toContainText('Clip 1')

  // Move by dragging to an empty cell of the same track: the same clip, one undo.
  let gesture = await drag(page, origin, at(page, 0, 2))
  await expect(at(page, 0, 2)).toHaveAttribute('data-drop', 'move')
  await expect(page.getByTestId('clip-ghost')).toBeVisible()
  await gesture.drop()
  await expect(at(page, 0, 2)).toHaveAttribute('data-clip', clipId)
  await expect(origin).toHaveAttribute('data-state', 'empty')
  await expect(at(page, 0, 2)).toHaveAttribute('data-state', 'idle')
  await page.keyboard.press('Control+z')
  await expect(origin).toHaveAttribute('data-clip', clipId)
  await expect(at(page, 0, 2)).toHaveAttribute('data-state', 'empty')

  // Alt + drag copies: an independent clip with the same notes; the source stays; one undo.
  gesture = await drag(page, origin, at(page, 0, 1), 'Alt')
  await expect(at(page, 0, 1)).toHaveAttribute('data-drop', 'copy')
  await gesture.drop()
  await expect(at(page, 0, 1)).toHaveAttribute('data-clip', /.+/)
  await expect(origin).toHaveAttribute('data-clip', clipId)
  const copyId = (await clipOf(at(page, 0, 1)))!
  expect(copyId).not.toBe(clipId)
  expect((await view()).tracks[midi[0]!]!.clips[copyId]!.content.midi!.notes).toHaveLength(1)
  // Nothing launched by the drops.
  await expect(origin).toHaveAttribute('data-state', 'idle')
  // Dropping on an occupied cell: no indicator, nothing sent (the history keeps its last entry).
  gesture = await drag(page, origin, at(page, 0, 1))
  await expect(at(page, 0, 1)).not.toHaveAttribute('data-drop', /.+/)
  await gesture.drop()
  await expect(origin).toHaveAttribute('data-clip', clipId)
  await expect(at(page, 0, 1)).toHaveAttribute('data-clip', copyId)
  await page.keyboard.press('Control+z')
  await expect(at(page, 0, 1)).toHaveAttribute('data-state', 'empty')
  await expect(origin).toHaveAttribute('data-clip', clipId)

  // To another MIDI track: moves it (same clip, other track); Alt copies. To an audio track: refused.
  gesture = await drag(page, origin, at(page, 1, 0))
  await expect(at(page, 1, 0)).toHaveAttribute('data-drop', 'move')
  await gesture.drop()
  await expect(at(page, 1, 0)).toHaveAttribute('data-clip', clipId)
  await expect(origin).toHaveAttribute('data-state', 'empty')
  expect((await view()).tracks[midi[1]!]!.clips[clipId]).toBeTruthy()
  await page.keyboard.press('Control+z')
  await expect(origin).toHaveAttribute('data-clip', clipId)
  gesture = await drag(page, origin, at(page, 1, 3), 'Alt')
  await gesture.drop()
  await expect(at(page, 1, 3)).toHaveAttribute('data-clip', /.+/)
  await expect(origin).toHaveAttribute('data-clip', clipId)
  await page.keyboard.press('Control+z')
  await expect(at(page, 1, 3)).toHaveAttribute('data-state', 'empty')
  gesture = await drag(page, origin, at(page, 2, 0))
  await expect(at(page, 2, 0)).not.toHaveAttribute('data-drop', /.+/)
  await gesture.drop()
  await expect(origin).toHaveAttribute('data-clip', clipId)
  await expect(at(page, 2, 0)).toHaveAttribute('data-state', 'empty')

  // Escape during a drag cancels it.
  gesture = await drag(page, origin, at(page, 0, 3))
  await page.keyboard.press('Escape')
  await gesture.drop()
  await expect(origin).toHaveAttribute('data-clip', clipId)
  await expect(at(page, 0, 3)).toHaveAttribute('data-state', 'empty')

  // Ctrl + C then Ctrl + V copies into the focused empty cell, in the same track or another one.
  await origin.focus()
  await page.keyboard.press('Control+c')
  await at(page, 0, 2).focus()
  await page.keyboard.press('Control+v')
  await expect(at(page, 0, 2)).toHaveAttribute('data-clip', /.+/)
  await at(page, 1, 1).focus()
  await page.keyboard.press('Control+v')
  await expect(at(page, 1, 1)).toHaveAttribute('data-clip', /.+/)
  await expect(origin).toHaveAttribute('data-clip', clipId)
  await page.keyboard.press('Control+z')
  await expect(at(page, 1, 1)).toHaveAttribute('data-state', 'empty')
  await expect(at(page, 0, 2)).toHaveAttribute('data-clip', /.+/)
  // Pasting on an occupied cell does nothing.
  await at(page, 0, 2).focus()
  const before = await clipOf(at(page, 0, 2))
  await page.keyboard.press('Control+v')
  await expect(at(page, 0, 2)).toHaveAttribute('data-clip', before!)

  // A clip the arrangement plays (made there, then moved into the grid) stays in its track: no move target on
  // another track, but Alt (copy) offers one.
  await run(page, { type: 'clip.create', track: midi[0], at: { at: 'arrangement', start: 0 } })
  const placedId = Object.entries((await view()).tracks[midi[0]!]!.clips).find(([, c]) => c.slot === null)![0]
  await run(page, { type: 'clip.move', clip: placedId, slot: scenes[3] })
  const placed = at(page, 0, 3)
  await expect(placed).toHaveAttribute('data-clip', placedId)
  gesture = await drag(page, placed, at(page, 1, 2))
  await expect(at(page, 1, 2)).not.toHaveAttribute('data-drop', /.+/)
  await page.keyboard.down('Alt')
  await expect(at(page, 1, 2)).toHaveAttribute('data-drop', 'copy')
  await page.keyboard.up('Alt')
  await expect(at(page, 1, 2)).not.toHaveAttribute('data-drop', /.+/)
  await gesture.drop()
  await expect(placed).toHaveAttribute('data-clip', placedId)
  await expect(at(page, 1, 2)).toHaveAttribute('data-state', 'empty')
  // Within its own track it moves.
  gesture = await drag(page, placed, at(page, 0, 1))
  await expect(at(page, 0, 1)).toHaveAttribute('data-drop', 'move')
  await gesture.drop()
  await expect(at(page, 0, 1)).toHaveAttribute('data-clip', placedId)

  expect(seen.errors).toEqual([])
})
