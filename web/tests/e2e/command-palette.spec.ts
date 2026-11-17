import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #108: the command palette (Ctrl+K) lists the catalogue's commands, finds them by typing and runs the one chosen through
// the dispatcher: a command without input runs at once, one with input asks for its fields first, and what the project
// refuses is said in the form.

test.use({ storageState: { cookies: [], origins: [] } })
test.describe.configure({ timeout: 90_000 })

const palette = (page: Page) => page.getByTestId('palette-dialog')
const search = (page: Page) => page.getByTestId('palette-search')

async function start(page: Page) {
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

test.describe('the command palette', () => {
  test('Ctrl+K opens a modal dialog on the search field, Escape closes it and gives the focus back', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await expect(palette(page)).toBeHidden()
    await page.keyboard.press('Control+k')
    await expect(palette(page)).toBeVisible()
    await expect(palette(page)).toHaveAttribute('aria-modal', 'true')
    await expect(palette(page)).toHaveAccessibleName('Commands')
    await expect(search(page)).toBeFocused()
    // Every command is listed up to the page's size, and the count says how many there are.
    await expect(page.getByTestId('palette-status')).toContainText(/50 of \d+ commands/)
    await page.keyboard.press('Escape')
    await expect(palette(page)).toBeHidden()
    // In a text field the key is left alone.
    await page.getByTestId('transport-tempo').focus()
    await page.keyboard.press('Control+k')
    await expect(palette(page)).toBeHidden()
    expect(errors).toEqual([])
  })

  test('a command without input runs at once from the keyboard', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('play')
    await expect(page.locator('[role="option"]').first()).toHaveAttribute('data-command', 'transport.play')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toBeHidden()
    await expect(page.getByTestId('transport-play')).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.press('Control+k')
    await search(page).fill('transport stop')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('transport-play')).toHaveAttribute('aria-pressed', 'false')
    expect(errors).toEqual([])
  })

  test('a command with input asks for it, says what is wrong, then runs and can be undone', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    const before = await page.getByTestId('project-name').innerText()
    await page.keyboard.press('Control+k')
    await search(page).fill('project.rename')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowUp')
    await page.keyboard.press('Enter')
    const form = page.getByTestId('palette-form')
    await expect(form).toBeVisible()
    await expect(form.locator('[data-field]').first()).toBeFocused()
    // The field starts on the current name; emptied, it is said, and nothing ran.
    await expect(form.locator('[data-field]').first()).toHaveValue(before)
    await form.locator('[data-field]').first().fill('')
    await page.getByTestId('palette-run').click()
    await expect(form.getByRole('alert')).toContainText('needs a value')
    await form.locator('[data-field]').first().fill('Palette song')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toBeHidden()
    await expect(page.getByTestId('project-name')).toContainText('Palette song')
    // It went through the dispatcher: the history has it, and Ctrl+Z takes it back.
    await page.keyboard.press('Control+z')
    await expect(page.getByTestId('project-name')).toHaveText(before)
    expect(errors).toEqual([])
  })

  test('a choice field offers the schema options, and Back returns to the list', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('view.set_theme')
    await page.keyboard.press('Enter')
    const select = page.getByTestId('palette-form').locator('select')
    await expect(select.locator('option')).toHaveText(['', 'system', 'dark', 'light'])
    await page.getByTestId('palette-back').click()
    await expect(search(page)).toBeFocused()
    await expect(page.getByTestId('palette-list')).toBeVisible()
  })

  test('before the audio starts a command that cannot run says so and the palette stays open', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    // The page is ready for keys once its engine button is drawn (the audio is not started).
    await expect(page.getByTestId('engine-state').getByRole('button')).toBeVisible()
    await page.keyboard.press('Control+k')
    await expect(palette(page)).toBeVisible()
    await search(page).fill('transport.play')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('palette-refused')).toContainText('Start the audio first')
    await expect(palette(page)).toBeVisible()
    // Typing again clears it.
    await search(page).fill('transport.stop')
    await expect(page.getByTestId('palette-refused')).toHaveCount(0)
    // A command that asks for input says it in its form too.
    await search(page).fill('project.rename')
    await page.keyboard.press('Enter')
    await page.getByTestId('palette-form').locator('[data-field]').first().fill('x')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('palette-refused')).toContainText('Start the audio first')
  })

  test('a destructive command warns in its form, and an empty level is never null', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('project.new')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('palette-destructive')).toContainText('replaces the project')
    await page.getByTestId('palette-back').click()
    await search(page).fill('mixer.set_volume')
    await page.keyboard.press('Enter')
    await page.getByTestId('palette-run').click()
    await expect(page.getByTestId('palette-form').getByRole('alert').first()).toContainText('needs a value')
    await expect(palette(page)).toBeVisible()
  })

  test('a query with no match says so', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('zzzzqqqq')
    await expect(page.getByTestId('palette-status')).toHaveText('No command matches.')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toBeVisible()
  })

  test('a command that has a shortcut shows its keys in the list, and the ones without show none (#109)', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('history.undo')
    const undo = page.locator('[role="option"][data-command="history.undo"]')
    await expect(undo.getByTestId('palette-keys')).toHaveText('Ctrl+Z')
    await search(page).fill('transport.stop')
    await expect(page.locator('[role="option"][data-command="transport.stop"]')).toBeVisible()
    await expect(page.getByTestId('palette-keys')).toHaveCount(0)
  })
})

const RULER = 28
const LANE = 44
const BAR = 3840
type Block = { id: string; row: number; start: number; length: number; clip: string }
const blocks = async (page: Page): Promise<Block[]> => JSON.parse((await page.getByTestId('arrangement').getAttribute('data-blocks')) ?? '[]')

/** A clip of one bar by a double click on the first track's row, at the first bar, then the focus on it. */
async function firstClip(page: Page) {
  await expect.poll(async () => Number(await page.getByTestId('arr-stage').getByTestId('timegrid').getAttribute('data-px-per-tick'))).toBeGreaterThan(0)
  await page.getByTestId('arr-zoom-out').click()
  await page.getByTestId('arr-zoom-out').click()
  const stage = page.getByTestId('arr-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await stage.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  await page.mouse.dblclick(box.x + (BAR / 2) * ppt, box.y + RULER + LANE / 2)
  await expect.poll(async () => (await blocks(page)).length).toBe(1)
  const block = (await blocks(page))[0]!
  await page.locator(`[data-testid=arr-item][data-block="${block.id}"]`).focus()
  return block
}

test.describe('the palette for a musician (#926)', () => {
  for (const [lang, query, label, goTo, goToQuery] of [
    ['en', 'lengthen', 'Lengthen to…', 'Go to', 'go to'],
    ['fr', 'allonger', 'Allonger jusqu’à…', 'Aller à', 'aller à'],
  ] as const) {
    test(`in ${lang}: the gestures are found by their names and run like their keys, on the clip that had the focus`, async ({ page }) => {
      const { errors } = watch(page)
      await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
      await page.setViewportSize({ width: 1440, height: 900 })
      await page.goto('/app')
      await start(page)
      await firstClip(page)
      await page.keyboard.press('Control+k')
      await search(page).fill(query)
      const option = page.locator('[role="option"]').first()
      await expect(option).toContainText(label)
      // No agent name on the line, no accent grave in its summary.
      await expect(option.locator('.name')).toHaveCount(0)
      await expect(option).not.toContainText('`')
      await expect(option.getByTestId('palette-keys')).toHaveText('Ctrl+Shift+L')
      await page.keyboard.press('Enter')
      await expect(palette(page)).toBeHidden()
      // The dialog of the shortcut opens, for the clip that was focused before the palette.
      await expect(page.getByTestId('lengthen-dialog')).toBeVisible()
      await page.getByTestId('lengthen-field').fill('5')
      await page.keyboard.press('Enter')
      await expect.poll(async () => (await blocks(page)).map((b) => b.start + b.length)[0]!).toBe(5 * BAR)

      // "Go to" opens its own field.
      await page.keyboard.press('Control+k')
      await search(page).fill(goToQuery)
      await expect(page.locator('[role="option"]').first()).toContainText(goTo)
      await page.keyboard.press('Enter')
      await expect(page.getByRole('dialog', { name: goTo })).toBeVisible()
      await page.keyboard.press('Escape')
      expect(errors).toEqual([])
    })
  }

  test('Step entry and Whole song run in their views, from another view', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await firstClip(page)
    const zoom = async () => Number(await page.getByTestId('arr-stage').getByTestId('timegrid').getAttribute('data-px-per-tick'))
    const zoomed = await zoom()
    await page.keyboard.press('Control+k')
    await search(page).fill('whole song')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toBeHidden()
    // The timeline fitted the song: its zoom changed.
    await expect.poll(zoom).not.toBe(zoomed)
    await page.keyboard.press('Control+k')
    await search(page).fill('step entry')
    await expect(page.locator('[role="option"]').first().getByTestId('palette-keys')).toHaveText('Alt+E')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('roll-step-entry')).toHaveAttribute('aria-pressed', 'true')
    expect(errors).toEqual([])
  })

  test('New clip, Add a marker and the shortcut list run from the palette', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await expect(page.getByTestId('arrangement')).toBeVisible()
    const run = async (query: string) => {
      await page.keyboard.press('Control+k')
      await search(page).fill(query)
      await page.keyboard.press('Enter')
      await expect(palette(page)).toBeHidden()
    }
    await run('new clip at the playhead')
    await expect.poll(async () => (await blocks(page)).length).toBe(1)
    await run('marker at the playhead')
    await expect(page.getByTestId('arr-marker')).toHaveCount(1)
    await run('keyboard shortcuts')
    await expect(page.getByTestId('shortcuts-dialog')).toBeVisible()
    await page.keyboard.press('Escape')
    expect(errors).toEqual([])
  })

  for (const [lang, transpose, semitones, uses, resize, end, to] of [
    ['en', 'transpose notes', 'Semitones', 'On the 3 selected notes.', 'resize a placement', 'end', 'To (bar.beat)'],
    ['fr', 'transposer', 'Demi-tons', 'Sur les 3 notes sélectionnées.', 'redimensionner un placement', 'end', 'À (mesure.temps)'],
  ] as const) {
    test(`in ${lang}: transposes the selected notes of the open clip and ends the selected placement at a bar, with no identifier typed (#926)`, async ({
      page,
    }) => {
      test.setTimeout(120_000)
      const { errors } = watch(page)
      await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
      await page.setViewportSize({ width: 1400, height: 900 })
      await page.goto('/app')
      await start(page)
      await page.getByTestId('panel-piano-roll').click()
      await page.getByTestId('roll-create-clip').click()
      const roll = page.getByTestId('piano-roll')
      await expect(roll).toHaveAttribute('data-note-count', '0')
      await expect.poll(async () => Number(await page.getByTestId('roll-stage').getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
      const stage = page.getByTestId('roll-stage')
      const box = (await stage.boundingBox())!
      const ppt = Number(await page.getByTestId('roll-stage').getByTestId('timegrid').getAttribute('data-px-per-tick'))
      const scrollY = Number(await stage.getAttribute('data-scroll-y'))
      const at = (tick: number, pitch: number) => [box.x + tick * ppt, box.y + 28 + (127 - pitch) * 16 - scrollY + 8] as const
      for (const [tick, pitch] of [
        [200, 60],
        [1100, 62],
        [2100, 64],
      ] as const)
        await page.mouse.click(...at(tick, pitch))
      const pitches = async () => (JSON.parse((await roll.getAttribute('data-notes')) ?? '[]') as { pitch: number }[]).map((n) => n.pitch)
      await expect.poll(pitches).toEqual([60, 62, 64])
      await page.getByTestId('roll-stage').getByTestId('timegrid').focus()
      await page.keyboard.press('Control+a')
      await expect(roll).toHaveAttribute('data-selected-count', '3')

      await page.keyboard.press('Control+k')
      await search(page).fill(transpose)
      await page.keyboard.press('Enter')
      const form = page.getByTestId('palette-form')
      await expect(form).toBeVisible()
      // Only the semitones are asked, labelled in the language of the interface; no identifier, no JSON.
      await expect(form.locator('[data-field]')).toHaveCount(1)
      await expect(form.locator('label')).toContainText(semitones)
      await expect(page.getByTestId('palette-uses').filter({ hasText: uses })).toHaveCount(1)
      await form.locator('[data-field]').fill('2')
      await page.keyboard.press('Enter')
      await expect(palette(page)).toBeHidden()
      await expect.poll(pitches).toEqual([62, 64, 66])

      // The clip is placed in the timeline: select it there, then end it at bar 5 (a position typed as "Go to").
      const block = (await blocks(page))[0]!
      expect(block.start).toBe(0)
      const arr = (await page.getByTestId('arr-stage').boundingBox())!
      const arrPpt = Number(await page.getByTestId('arr-stage').getByTestId('timegrid').getAttribute('data-px-per-tick'))
      await page.mouse.click(arr.x + (BAR / 2) * arrPpt, arr.y + RULER + LANE / 2)
      await expect(page.locator(`[data-testid=arr-item][data-block="${block.id}"]`)).toHaveAttribute('aria-selected', 'true')
      await page.keyboard.press('Control+k')
      await search(page).fill(resize)
      await page.keyboard.press('Enter')
      await expect(form.locator('[data-testid="palette-needs"]')).toHaveCount(0)
      await expect(form.locator('label').filter({ hasText: to })).toHaveCount(1)
      await form.locator('select[data-field]').first().selectOption(end)
      await form.locator('input[data-field]').fill('5')
      await page.keyboard.press('Enter')
      await expect(palette(page)).toBeHidden()
      await expect.poll(async () => (await blocks(page)).map((b) => b.start + b.length)[0]!).toBe(4 * BAR)
      expect(errors).toEqual([])
    })
  }

  test('a command that needs a selection says so, without a field to type an identifier or JSON in (#926)', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('transpose notes')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('palette-needs')).toContainText('first')
    await expect(page.getByTestId('palette-form').locator('[data-field]')).toHaveCount(0)
    await expect(page.getByTestId('palette-run')).toBeDisabled()
    await page.getByTestId('palette-back').click()
    await search(page).fill('rename the project')
    await page.keyboard.press('Enter')
    // The field starts on the current name.
    await expect(page.getByTestId('palette-form').locator('[data-field]').first()).not.toHaveValue('')
  })

  test('lists no command for the agents until a > is typed, and shows Space on Play', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('restore')
    await expect(page.getByTestId('palette-status')).toHaveText('No command matches.')
    await search(page).fill('>restore')
    await expect(page.locator('[role="option"][data-command="track.restore"]')).toBeVisible()
    await expect(page.locator('[role="option"][data-command="track.restore"] .name')).toHaveText('track.restore')
    await search(page).fill('play')
    await expect(page.locator('[role="option"][data-command="transport.play"]').getByTestId('palette-keys')).toHaveText('Space')
    await search(page).fill('ctrl')
    await expect(page.locator('[role="option"][data-command="history.undo"]')).toBeVisible()
  })
})
