import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #953: key changes placed at a bar. The transport's key panel adds one at the playhead's bar and edits the key in force; the ruler
// of the arrangement shows each as a button (click: the playhead goes there; Alt+arrows and a drag move it by bars; Delete removes
// it); the transport's button shows the key at the playhead; the piano roll names and shades the notes from the key in force where
// the placement it was opened from starts. Real engine and project core; the console stays clean.

test.use({ storageState: { cookies: [], origins: [] } })
test.describe.configure({ timeout: 120_000 })

const BAR = 3840
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
type Block = { id: string; start: number; clip: string }
const blocks = async (page: Page): Promise<Block[]> => JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')
const button = (page: Page) => page.getByTestId('transport-scale')
const changes = (page: Page) => page.getByTestId('arr-key-change')

async function start(page: Page, lang: 'en' | 'fr' = 'en') {
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page, 'Web')
  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function', 'the dev server only')
}

async function firstTrack(page: Page): Promise<string> {
  return page.evaluate(
    () => Object.keys((window as unknown as { __faulkView(): { project: { tracks: Record<string, unknown> } } }).__faulkView().project.tracks)[0]!,
  )
}

/** D major from bar 1, one clip with a B♭ placed at bar 1 and bar 5 (the same clip, twice). */
async function song(page: Page) {
  await run(page, { type: 'project.set_scale', root: 2, mode: 'major' })
  await page.getByTestId('track-header').first().click()
  await run(page, { type: 'clip.create', track: await firstTrack(page), at: { at: 'arrangement', start: 0 }, length: BAR })
  const clip = (await blocks(page))[0]!.clip
  await run(page, { type: 'clip.notes.add', clip, notes: [{ start: 0, pitch: 70, length: BAR / 4, velocity: 100, cents: 0, chance: 100, repeat: 1 }] })
  await run(page, { type: 'arrangement.place', clip, start: 4 * BAR, copy: false })
  await expect.poll(async () => (await blocks(page)).length).toBe(2)
  return (await blocks(page)).sort((a, b) => a.start - b.start)
}

const seek = async (page: Page, tick: number) => {
  await run(page, { type: 'transport.seek', tick })
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-position', String(tick))
}

test('add a key change from the key panel: the button follows the playhead, the panel edits the key in force, undo takes it back', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await song(page)
  await expect(button(page)).toHaveText('D')

  // At bar 1 there is nothing to add: the first bar's key is the starting key.
  await button(page).click()
  await expect(page.getByTestId('scale-from')).toHaveText('Starting key')
  await expect(page.getByTestId('scale-add-change')).toHaveCount(0)
  await page.keyboard.press('Escape')

  // At bar 5 it can be added, in the key in force there (D major), then the panel edits that change.
  await seek(page, 4 * BAR)
  await button(page).click()
  await page.getByTestId('scale-add-change').click()
  await expect(changes(page)).toHaveCount(1)
  await expect(changes(page)).toHaveAttribute('data-tick', String(4 * BAR))
  await expect(page.getByTestId('scale-from')).toHaveText('Key from bar 5')
  await page.getByTestId('scale-root').selectOption({ label: 'F' })
  await expect(changes(page)).toHaveText('F')
  await expect(button(page)).toHaveText('F')
  // The playhead leaves the panel's key: the button shows the key at the playhead, the panel keeps editing the one it opened on.
  await seek(page, 0)
  await expect(button(page)).toHaveText('D')
  await expect(page.getByTestId('scale-from')).toHaveText('Key from bar 5')
  await page.keyboard.press('Escape')
  await seek(page, 5 * BAR + 100)
  await expect(button(page)).toHaveText('F')
  await expect(button(page)).toHaveAttribute('data-mode', 'major')

  // The project's own scale did not change, and the engine took the change (no console error).
  await run(page, { type: 'history.undo' })
  await run(page, { type: 'history.undo' })
  await expect(changes(page)).toHaveCount(0)
  await seek(page, 5 * BAR)
  await expect(button(page)).toHaveText('D')
  expect(errors).toEqual([])
})

test('the ruler: a key change is moved by bars with the keyboard and a drag, removed with Delete, and undo restores each', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await song(page)
  await run(page, { type: 'arrangement.key_change.set', tick: 4 * BAR, root: 5, mode: 'major' })
  await expect(changes(page)).toHaveCount(1)
  const change = changes(page).first()
  await expect(change).toHaveAttribute('aria-label', 'Key change at bar 5: F major')

  // A click puts the playhead there.
  await change.click()
  await expect(page.getByTestId('arrangement')).toHaveAttribute('data-position', String(4 * BAR))

  // Alt+Right moves it a bar later, Alt+Left back.
  await change.focus()
  await page.keyboard.press('Alt+ArrowRight')
  await expect(changes(page).first()).toHaveAttribute('data-tick', String(5 * BAR))
  await changes(page).first().focus()
  await page.keyboard.press('Alt+ArrowLeft')
  await expect(changes(page).first()).toHaveAttribute('data-tick', String(4 * BAR))

  // A drag of two bars drops it on bar 7.
  const ppt = Number(await page.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const box = (await changes(page).first().boundingBox())!
  await page.mouse.move(box.x + 6, box.y + 6)
  await page.mouse.down()
  await page.mouse.move(box.x + 6 + (2 * BAR * ppt) / 2, box.y + 6, { steps: 4 })
  await page.mouse.move(box.x + 6 + 2 * BAR * ppt, box.y + 6, { steps: 4 })
  await page.mouse.up()
  await expect(changes(page).first()).toHaveAttribute('data-tick', String(6 * BAR))

  // Delete removes it; undo brings it back where it was, undo again puts it back on bar 5.
  await changes(page).first().focus()
  await page.keyboard.press('Delete')
  await expect(changes(page)).toHaveCount(0)
  await run(page, { type: 'history.undo' })
  await expect(changes(page).first()).toHaveAttribute('data-tick', String(6 * BAR))
  await run(page, { type: 'history.undo' })
  await expect(changes(page).first()).toHaveAttribute('data-tick', String(4 * BAR))

  // The right click offers the same actions.
  await changes(page).first().click({ button: 'right' })
  await expect(page.getByRole('menuitem', { name: 'One bar later' })).toBeVisible()
  await page.getByRole('menuitem', { name: 'Delete' }).click()
  await expect(changes(page)).toHaveCount(0)
  expect(errors).toEqual([])
})

test('the piano roll names and shades the notes from the key in force where the placement it was opened from starts', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const placed = await song(page)
  await run(page, { type: 'arrangement.key_change.set', tick: 4 * BAR, root: 5, mode: 'major' })
  await expect(changes(page)).toHaveCount(1)
  await page.getByTestId('panel-piano-roll').click()
  const roll = page.getByTestId('piano-roll')
  await expect(roll).toBeVisible()
  // The first placement (bar 1, D major: F♯ and C♯, no B♭): sharps.
  await expect(roll).toHaveAttribute('data-scale', '1,2,4,6,7,9,11')
  await expect(roll).toHaveAttribute('data-flats', 'false')

  // Opened from the placement at bar 5 (F major: B♭ in the key): the same note is a flat there.
  await page.click().catch(() => undefined)
  const second = page.locator(`[data-testid=arr-item][data-block="${placed[1]!.id}"]`)
  await second.focus()
  await page.keyboard.press('Enter')
  await expect(roll).toBeVisible()
  await expect(roll).toHaveAttribute('data-scale', '0,2,4,5,7,9,10')
  await expect(roll).toHaveAttribute('data-flats', 'true')
  expect(errors).toEqual([])
})

test('the key change texts are in French too', async ({ page }) => {
  const { errors } = watch(page)
  await start(page, 'fr')
  await song(page)
  await run(page, { type: 'arrangement.key_change.set', tick: 4 * BAR, root: 5, mode: 'major' })
  await expect(changes(page).first()).toHaveAttribute('aria-label', 'Changement de tonalité à la mesure 5 : Fa majeur')
  await seek(page, 4 * BAR)
  await button(page).click()
  await expect(page.getByTestId('scale-from')).toHaveText('Tonalité à partir de la mesure 5')
  await expect(page.getByTestId('scale-remove-change')).toHaveText('Retirer ce changement de tonalité')
  await page.getByTestId('scale-remove-change').click()
  await expect(changes(page)).toHaveCount(0)
  expect(errors).toEqual([])
})
