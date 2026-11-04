import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// PR-7 (#52): the history panel in a real browser. A note drag is one entry; each entry shows its
// label, who did it (you, or an agent) and when; undo and redo (buttons and Ctrl+Z) move entries
// between the done list and the undone one.

const ROW = 16
const RULER = 28

const roll = (page: Page) => page.getByTestId('piano-roll')
const entries = (page: Page) => page.getByTestId('history-entry')
const num = async (page: Page, testid: string, attribute: string) => Number(await page.getByTestId(testid).getAttribute(attribute))

async function spot(page: Page) {
  const box = (await page.getByTestId('roll-stage').boundingBox())!
  const ppt = await num(page, 'timegrid', 'data-px-per-tick')
  const scrollY = await num(page, 'roll-stage', 'data-scroll-y')
  return { x: (tick: number) => box.x + tick * ppt, y: (pitch: number) => box.y + RULER + (127 - pitch) * ROW - scrollY + ROW / 2 }
}

const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const run = (page: Page, command: object, author?: string) =>
  page.evaluate(([c, a]) => (window as unknown as { __faulkRun(c: object, a?: string): void }).__faulkRun(c as object, a as string | undefined), [
    command,
    author,
  ] as const)

/** The names of the done entries, newest first. */
const done = async (page: Page) =>
  entries(page).evaluateAll((rows) => rows.filter((r) => r.getAttribute('data-undone') === 'false').map((r) => r.getAttribute('data-name')))

test('history panel: a note drag is one entry, entries show author and time, undo and redo move them', async ({ page }) => {
  test.setTimeout(120_000)
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1400, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)

  // A note drawn: one entry.
  const g = await spot(page)
  await page.mouse.click(g.x(480), g.y(60))
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  await page.getByTestId('panel-history').click()
  await expect(entries(page).first()).toBeVisible()
  const before = await entries(page).count()
  expect(await done(page)).toContain('clip.notes.add')

  // The drag, with many pointer moves, is one entry more, not one per move.
  await page.getByTestId('panel-piano-roll').click()
  const h = await spot(page)
  const from = { x: h.x(480) + 6, y: h.y(60) }
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  for (let i = 1; i <= 12; i++) await page.mouse.move(from.x + i * 12, from.y - Math.floor(i / 4) * ROW)
  await page.mouse.up()
  await page.getByTestId('panel-history').click()
  await expect(entries(page)).toHaveCount(before + 1)
  const top = entries(page).first()
  await expect(top).toHaveAttribute('data-author', 'user')
  await expect(top).toContainText('You')
  await expect(top.locator('time')).toHaveText(/\d{2}:\d{2}:\d{2}/)

  // The dispatcher hook (to run a command as an agent) exists in the dev server only.
  if ((await hooks(page)) !== 'function') {
    expect(errors).toEqual([])
    return
  }

  // An agent's edit shows its name.
  await run(page, { type: 'project.rename', name: 'By the agent' }, 'agent:claude')
  await expect(entries(page).first()).toHaveAttribute('data-author', 'agent:claude')
  await expect(entries(page).first()).toContainText('claude')
  await expect(entries(page).first()).toContainText('Rename the project')
  // The author is back to the user for the next edit.
  await run(page, { type: 'project.rename', name: 'By me' })
  await expect(entries(page).first()).toHaveAttribute('data-author', 'user')

  // Ctrl+Z moves the newest entry to the undone ones; the redo button brings it back.
  const total = await entries(page).count()
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('history-entry').first()).toHaveAttribute('data-undone', 'true')
  await expect(entries(page)).toHaveCount(total)
  await page.getByTestId('history-redo').click()
  await expect(entries(page).first()).toHaveAttribute('data-undone', 'false')
  await page.getByTestId('history-undo').click()
  await expect(entries(page).first()).toHaveAttribute('data-undone', 'true')
  // A new edit drops what could be redone.
  await run(page, { type: 'project.rename', name: 'Fresh' })
  await expect(page.locator('[data-testid=history-entry][data-undone=true]')).toHaveCount(0)

  expect(errors).toEqual([])
})

test('history panel: French labels and an empty history', async ({ page }) => {
  const { errors } = watch(page)
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('panel-history').click()
  await expect(page.getByTestId('history')).toBeVisible()
  await run(page, { type: 'project.rename', name: 'Titre' }, 'agent:claude')
  await expect(entries(page).first()).toContainText('Renommer le projet')
  await expect(entries(page).first()).toContainText('claude')
  await run(page, { type: 'project.rename', name: 'Titre 2' }, 'agent:claude')
  await expect(entries(page).first().getByTestId('history-turn')).toHaveText('2 modifications en un tour')
  expect(errors).toEqual([])
})

// AG-3 (#99): an agent's consecutive edits are one turn, one row that one undo undoes; a user's gesture during the
// turn has its own row and ends the turn.
test("history panel: an agent's turn is one row that one undo undoes, and the user's gesture stays out of it", async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('panel-history').click()
  const name = page.getByTestId('project-name')
  const original = (await name.textContent())!.trim()

  await run(page, { type: 'project.rename', name: 'Agent one' }, 'agent:claude')
  await run(page, { type: 'project.rename', name: 'Agent two' }, 'agent:claude')
  const top = entries(page).first()
  await expect(top).toHaveAttribute('data-author', 'agent:claude')
  await expect(top).toHaveAttribute('data-edits', '2')
  await expect(top.getByTestId('history-turn')).toHaveText('2 edits in one turn')

  // The user's gesture in the middle of the turn: a row of its own; the agent's next edit is a new turn.
  const rows = await entries(page).count()
  await run(page, { type: 'project.rename', name: 'Mine' })
  await run(page, { type: 'project.rename', name: 'Agent three' }, 'agent:claude')
  await expect(entries(page)).toHaveCount(rows + 2)
  await expect(entries(page).nth(1)).toHaveAttribute('data-author', 'user')
  await expect(entries(page).first()).toHaveAttribute('data-author', 'agent:claude')
  await expect(entries(page).first().getByTestId('history-turn')).toHaveCount(0)

  // Undone in the order of application: the agent's new turn, the gesture, then the whole first turn at once.
  const undo = page.getByTestId('history-undo')
  await undo.click()
  await expect(name).toHaveText('Mine')
  await undo.click()
  await expect(name).toHaveText('Agent two')
  await undo.click()
  await expect(name).toHaveText(original)
  expect(errors).toEqual([])
})
